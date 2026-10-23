//! The four step functions: what a preset plays on sixteenth step `s`.
//!
//! Each one is the browser engine's own function of the same name, with Web Audio nodes turned
//! into [`VoiceSpec`]s. The comments that mattered there are kept, because they explain choices
//! that are easy to undo by accident -- why the EDM motif sits on those steps, why the scape
//! holds its degree across a preset change, why lo-fi pushes the second sixteenth late.
//!
//! One addition the browser could not make: every helper takes an `offset` in seconds. The
//! browser swung lo-fi by adding to the event time and dividing the swing tolerance by its 25 ms
//! poll; here a step's voices simply start later, on an exact sample, which is what makes the
//! limp deliberate rather than approximate.

use super::music::{
    Arrangement, LayerName, ParamName, Preset, AMB_CHORDS, AMB_ROOTS, EDM_CHORDS, EDM_MOTIF,
    EDM_ROOTS, LOFI_CHORDS, LOFI_PENTA, LOFI_ROOTS,
};
use super::rng::Mulberry32;
use super::synth::{Bus, Env, FilterSpec, VoiceSpec, Voices, Wave};

/// What a step did, for the visuals and for the tests: a kick is a flash, a note is a point in
/// the field. Counters rather than times, because the visuals only need to know that something
/// happened since they last looked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Events {
    pub kicks: u32,
    pub notes: u32,
    pub basses: u32,
    pub chords: u32,
    /// The most recent note, as midi and stereo position, for a comet in the field.
    pub midi: i32,
    pub pan: f32,
}

impl Events {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        self.kicks == 0 && self.notes == 0 && self.basses == 0 && self.chords == 0
    }

    /// Fold another step's events into this one, for the engine's per-block report.
    pub fn absorb(&mut self, other: &Events) {
        if other.is_empty() {
            return;
        }
        self.kicks += other.kicks;
        self.notes += other.notes;
        self.basses += other.basses;
        self.chords += other.chords;
        if other.notes > 0 || other.basses > 0 || other.chords > 0 {
            self.midi = other.midi;
            self.pan = other.pan;
        }
    }
}

/// The musical memory a running sequence carries: which degree the scape stands on, and the
/// stream every stochastic choice draws from.
pub struct Sequence {
    pub rng: Mulberry32,
    /// The degree the scape is on.
    pub degree: i32,
    /// The chord currently sounding, so a preset change can hand over on shared notes.
    pub chord: Option<[i32; 3]>,
    /// Set by a preset change: the next chord change keeps the degree instead of walking.
    pub hold_degree: bool,
}

impl Sequence {
    pub fn new(seed: u32) -> Self {
        Self {
            rng: Mulberry32::new(seed),
            degree: 0,
            chord: None,
            hold_degree: false,
        }
    }

    pub fn set_seed(&mut self, seed: u32) {
        self.rng = Mulberry32::new(seed);
    }

    /// Start a fresh preset's grid: the chord and the degree belong to the preset that made
    /// them, not to the one arriving.
    pub fn reset_grid(&mut self) {
        self.chord = None;
    }
}

/// The pan law the browser's `pluck` and `rhodes` use: pitch class places a note in the field,
/// so a note heard on the left is also the note drawn on the left.
fn pitch_pan(midi: i32, jitter: f32) -> f32 {
    (pitch_class_offset(midi) + jitter).clamp(-1., 1.)
}

fn pitch_class_offset(midi: i32) -> f32 {
    ((midi.rem_euclid(12) as f32) / 11. - 0.5) * 0.9
}

/// Play one sixteenth step. `step` is the step within the current preset's own grid, which is
/// what makes a preset change start the new material at its beginning while the master clock
/// keeps running underneath.
pub fn play_step(
    arrangement: &mut Arrangement,
    sequence: &mut Sequence,
    voices: &mut Voices,
    step: u32,
    events: &mut Events,
) {
    match arrangement.preset {
        Preset::Edm => step_edm(arrangement, sequence, voices, step, events),
        Preset::Ambient => step_ambient(arrangement, sequence, voices, step, events),
        Preset::Lofi => step_lofi(arrangement, sequence, voices, step, events),
        _ => step_scape(arrangement, sequence, voices, step, events),
    }
}

fn step_scape(
    arrangement: &mut Arrangement,
    sequence: &mut Sequence,
    voices: &mut Voices,
    step: u32,
    events: &mut Events,
) {
    if arrangement.preset.scape().is_none() {
        return;
    }
    let layers = arrangement.mix();
    let density = (arrangement.eff(ParamName::Density) * (0.5 + arrangement.energy)).min(1.);
    let brightness = arrangement.eff(ParamName::Brightness);
    let pulse = arrangement.eff(ParamName::Pulse);
    let pace = arrangement.eff(ParamName::Pace);

    if step.is_multiple_of(32) || sequence.chord.is_none() {
        // After a preset change the degree is already chosen for common tones with the outgoing
        // chord, so the first change of a new scene must not walk away from it.
        scape_chord_change(arrangement, sequence, voices, !sequence.hold_degree, events);
        sequence.hold_degree = false;
    }
    let chord = match sequence.chord {
        Some(chord) => chord,
        None => return,
    };

    if layers.drums > 0. && pulse > 0.1 && step.is_multiple_of(8) {
        // Higher top of the pitch drop with more pulse: a pulse change reads as a change in
        // attack rather than in level.
        kick(
            voices,
            65. + 50. * pulse,
            36.,
            0.45 + 0.5 * (1. - pace),
            (0.14 + 0.34 * pulse) * layers.drums,
            0.,
            events,
        );
    }

    if layers.melody > 0. && step.is_multiple_of(2) && sequence.rng.chance(density * 0.5) {
        let octave = if sequence.rng.chance(0.45) { 24 } else { 12 };
        let index = sequence.rng.below(3);
        let midi = chord[index] + octave;
        let jitter = sequence.rng.signed() * 0.16;
        let wave = if sequence.rng.chance(0.5) {
            Wave::Sine
        } else {
            Wave::Triangle
        };
        pluck(
            voices,
            midi,
            (0.028 + 0.05 * density) * layers.melody,
            0.7 + 1.6 * (1. - pace),
            wave,
            pitch_pan(midi, jitter),
            0.,
            events,
        );
    }
    if layers.melody > 0. && step % 16 == 8 && sequence.rng.chance(0.18 * brightness) {
        let midi = chord[0] + 36;
        pluck(
            voices,
            midi,
            0.018 * layers.melody,
            2.2,
            Wave::Sine,
            pitch_pan(midi, 0.),
            0.,
            events,
        );
    }
}

fn scape_chord_change(
    arrangement: &mut Arrangement,
    sequence: &mut Sequence,
    voices: &mut Voices,
    walk: bool,
    events: &mut Events,
) {
    let Some(scape) = arrangement.preset.scape() else {
        return;
    };
    let length = scape.degrees();
    if walk {
        let choices = [-2, -1, 1, 2];
        let by = choices[sequence.rng.below(4)];
        sequence.degree = (sequence.degree + by + length * 8) % length;
    }
    let chord = arrangement.scape_triad(sequence.degree);
    sequence.chord = Some(chord);

    let brightness = arrangement.eff(ParamName::Brightness);
    let pace = arrangement.eff(ParamName::Pace);
    let spb = arrangement.spb();
    let layers = arrangement.mix();

    if layers.pads > 0. {
        pad(
            voices,
            [chord[0] + 12, chord[1] + 12, chord[2] + 12],
            spb * 8. + 2.,
            0.05 * layers.pads,
            350. + 2200. * brightness,
            1. + 2.5 * (1. - pace),
            3.,
        );
        events.chords += 1;
    }
    if layers.bass > 0. {
        sub(
            voices,
            chord[0] - 12,
            spb * 8. + 2.,
            (0.1 + 0.1 * (1. - brightness)) * layers.bass,
            2.5,
        );
        events.basses += 1;
        events.midi = chord[0] - 12;
    }
}

fn step_edm(
    arrangement: &mut Arrangement,
    sequence: &mut Sequence,
    voices: &mut Voices,
    step: u32,
    events: &mut Events,
) {
    let bar = step / 16;
    let sb = step % 16;
    let layers = arrangement.mix();
    let section = (bar / 8) % 4;
    let section_multiplier = [0.6, 0.85, 1.0, 0.55][section as usize];
    let energy = arrangement.energy * section_multiplier;
    let chord = EDM_CHORDS[(bar % 4) as usize];
    let spb = arrangement.spb();
    let sixteenth = spb / 4.;

    if sb == 0 && layers.pads > 0. {
        pad(
            voices,
            chord,
            spb * 4.,
            0.05 * layers.pads,
            1100. + 600. * energy,
            0.08,
            0.6,
        );
        events.chords += 1;
    }

    if layers.drums > 0. {
        if step.is_multiple_of(4) && energy >= 0.28 {
            kick(voices, 150., 44., 0.3, 0.95 * layers.drums, 0., events);
        }
        // Hats alternate sides, the clap stays centred, the riser opens wide.
        if step % 4 == 2 && energy >= 0.48 {
            noise_hit(
                voices,
                sequence,
                FilterSpec::highpass(7500., 1.),
                if sb % 8 < 4 { -0.5 } else { 0.5 },
                0.16 * layers.drums,
                0.05,
                0.,
            );
        }
        if (sb == 4 || sb == 12) && energy >= 0.62 {
            noise_hit(
                voices,
                sequence,
                FilterSpec::bandpass(1800., 1.2),
                0.,
                0.4 * layers.drums,
                0.13,
                0.,
            );
        }
        if section == 3 && bar % 8 == 7 {
            let jitter = sequence.rng.signed() * 0.7;
            noise_hit(
                voices,
                sequence,
                FilterSpec::bandpass(1900., 1.5),
                jitter,
                (0.12 + 0.02 * sb as f32) * layers.drums,
                0.08,
                0.,
            );
        }
    }

    if layers.bass > 0. && step.is_multiple_of(2) && energy >= 0.42 {
        bass(
            voices,
            EDM_ROOTS[(bar % 4) as usize],
            sixteenth * 2. * 0.9,
            (if step.is_multiple_of(4) { 0.3 } else { 0.22 }) * layers.bass,
            0.,
            events,
        );
    }

    if layers.melody > 0. && energy >= 0.38 {
        let phrase = step % 32;
        if let Some((_, midi, length)) = EDM_MOTIF.iter().find(|(at, _, _)| *at == phrase) {
            lead(
                voices,
                *midi,
                sixteenth * *length as f32 * 0.95,
                0.16 * layers.melody,
                events,
            );
            if energy >= 0.85 {
                lead(
                    voices,
                    *midi + 12,
                    sixteenth * *length as f32 * 0.95,
                    0.07 * layers.melody,
                    events,
                );
            }
        }
    }

    if layers.melody > 0. && energy >= 0.78 {
        let tones = [chord[0] + 12, chord[1] + 12, chord[2] + 12, chord[1] + 24];
        let midi = tones[(step % 4) as usize];
        pluck(
            voices,
            midi,
            0.07 * layers.melody,
            0.14,
            Wave::Square,
            pitch_pan(midi, 0.),
            0.,
            events,
        );
    }
}

fn step_ambient(
    arrangement: &mut Arrangement,
    sequence: &mut Sequence,
    voices: &mut Voices,
    step: u32,
    events: &mut Events,
) {
    let chord_index = ((step / 32) % 4) as usize;
    let layers = arrangement.mix();
    let chord = AMB_CHORDS[chord_index];
    let energy = arrangement.energy;
    let spb = arrangement.spb();

    if step.is_multiple_of(32) {
        if layers.pads > 0. {
            pad(
                voices,
                chord,
                spb * 8. + 1.,
                0.055 * layers.pads,
                750. + 500. * energy,
                2.,
                3.,
            );
            events.chords += 1;
        }
        if layers.bass > 0. {
            sub(
                voices,
                AMB_ROOTS[chord_index] - 12,
                spb * 8.,
                (0.16 * energy + 0.04) * layers.bass,
                2.5,
            );
            events.basses += 1;
        }
    }
    if layers.drums > 0. && step % 64 == 16 && energy >= 0.35 {
        kick(voices, 90., 32., 1.1, 0.5 * layers.drums, 0., events);
    }

    if layers.melody > 0. && step.is_multiple_of(2) && sequence.rng.chance(0.3 + 0.45 * energy) {
        let octave = if sequence.rng.chance(0.4) { 24 } else { 12 };
        let index = sequence.rng.below(3);
        let midi = chord[index] + octave;
        let jitter = sequence.rng.signed() * 0.16;
        let wave = if sequence.rng.chance(0.5) {
            Wave::Sine
        } else {
            Wave::Triangle
        };
        pluck(
            voices,
            midi,
            (0.05 + 0.05 * energy) * layers.melody,
            0.9,
            wave,
            pitch_pan(midi, jitter),
            0.,
            events,
        );
    }
    if layers.melody > 0. && step % 16 == 8 && sequence.rng.chance(0.3 * energy) {
        let midi = chord[0] + 36;
        pluck(
            voices,
            midi,
            0.025 * layers.melody,
            1.6,
            Wave::Sine,
            pitch_pan(midi, 0.),
            0.,
            events,
        );
    }
}

fn step_lofi(
    arrangement: &mut Arrangement,
    sequence: &mut Sequence,
    voices: &mut Voices,
    step: u32,
    events: &mut Events,
) {
    let bar = step / 16;
    let sb = step % 16;
    let layers = arrangement.mix();
    let chord = LOFI_CHORDS[(bar % 4) as usize];
    let energy = arrangement.energy;
    let spb = arrangement.spb();
    // The second sixteenth of each beat lands late: that is the whole feel of the preset.
    let swing = if step % 4 == 2 { spb * 0.14 } else { 0. };

    if (sb == 0 || sb == 10) && layers.pads > 0. {
        for (index, midi) in chord.iter().enumerate() {
            // The browser staggers the chord by 12 ms per tone, which is what makes it read as
            // a Rhodes rather than a block chord.
            rhodes(
                voices,
                *midi,
                0.075 * layers.pads,
                1.6,
                sequence.rng.signed() * 4.,
                swing + index as f32 * 0.012,
            );
        }
        events.chords += 1;
    }

    if layers.drums > 0. {
        if (sb == 0 || sb == 7) && energy >= 0.3 {
            kick(voices, 95., 42., 0.22, 0.5 * layers.drums, swing, events);
        }
        if (sb == 4 || sb == 12) && energy >= 0.3 {
            noise_hit(
                voices,
                sequence,
                FilterSpec::lowpass(3800., 3800., 0.8),
                0.,
                0.18 * layers.drums,
                0.16,
                swing,
            );
            events.basses += 1;
            events.midi = 50;
        }
        if step % 4 == 2 && energy >= 0.55 {
            noise_hit(
                voices,
                sequence,
                FilterSpec::highpass(8200., 1.),
                if sb % 8 < 4 { -0.45 } else { 0.45 },
                0.05 * layers.drums,
                0.04,
                swing,
            );
        }
    }

    if layers.bass > 0. && (sb == 0 || sb == 8) && energy >= 0.35 {
        bass(
            voices,
            LOFI_ROOTS[(bar % 4) as usize] - 12,
            0.5,
            0.26 * layers.bass,
            swing,
            events,
        );
    }

    if layers.melody > 0. && step.is_multiple_of(4) && sequence.rng.chance(0.22 + 0.3 * energy) {
        let index = sequence.rng.below(5);
        let midi = LOFI_PENTA[index];
        rhodes(
            voices,
            midi,
            0.05 * layers.melody,
            1.1,
            sequence.rng.signed() * 4.,
            swing + 0.02,
        );
        events.chords += 1;
    }
    if layers.get(LayerName::Texture) > 0. && sequence.rng.chance(0.045) {
        let jitter = sequence.rng.signed() * 0.8;
        noise_hit(
            voices,
            sequence,
            FilterSpec::bandpass(3000., 8.),
            jitter,
            0.04 * layers.get(LayerName::Texture),
            0.015,
            swing,
        );
    }
}

// ---- voices ---------------------------------------------------------------------------------
//
// `offset` is the step's swing, in seconds, and is zero for every preset that does not swing.

fn kick(
    voices: &mut Voices,
    from_hz: f32,
    to_hz: f32,
    duration: f32,
    amplitude: f32,
    offset: f32,
    events: &mut Events,
) {
    voices.start(
        VoiceSpec {
            hz: from_hz,
            glide_to: Some(to_hz),
            // A third of the note: enough for the drop to be heard as pitch, short enough that the
            // rest of the note is the body rather than the fall.
            glide_seconds: duration * 0.35,
            ..VoiceSpec::new(
                Wave::Sine,
                60,
                Env::Decay {
                    attack: 0.002,
                    total: duration,
                },
                amplitude,
            )
        }
        .on(Bus::Dry)
        .delayed(offset),
    );
    events.kicks += 1;
}

/// The sidechained bass: a saw through a lowpass that closes over the note.
fn bass(
    voices: &mut Voices,
    midi: i32,
    duration: f32,
    amplitude: f32,
    offset: f32,
    events: &mut Events,
) {
    voices.start(
        VoiceSpec::new(
            Wave::Saw,
            midi,
            Env::Decay {
                attack: 0.005,
                total: duration,
            },
            amplitude,
        )
        .on(Bus::Duck)
        .delayed(offset)
        .filter(FilterSpec::lowpass(650., 220., 0.9)),
    );
    events.basses += 1;
    events.midi = midi;
}

/// A sub: a sine that swells in over seconds and falls away. The scapes' bottom end.
fn sub(voices: &mut Voices, midi: i32, duration: f32, amplitude: f32, attack: f32) {
    voices.start(
        VoiceSpec::new(
            Wave::Sine,
            midi,
            Env::Hold {
                attack,
                hold_until: (duration - 1.5).max(attack),
                total: duration,
            },
            amplitude,
        )
        .on(Bus::Dry),
    );
}

/// A pad: two detuned saws per chord tone, hard left and right, each tone leaning a little
/// further out so a triad occupies the field rather than one point.
fn pad(
    voices: &mut Voices,
    chord: [i32; 3],
    duration: f32,
    amplitude: f32,
    cutoff: f32,
    attack: f32,
    release: f32,
) {
    let env = Env::Hold {
        attack,
        hold_until: (duration - release).max(attack),
        total: duration + 0.05,
    };
    let last = chord.len() as f32 - 1.;
    for (index, midi) in chord.iter().enumerate() {
        let lean = (index as f32 / last - 0.5) * 0.2;
        for (detune, pan) in [(-7., 0.55), (7., -0.55)] {
            voices.start(
                VoiceSpec::new(Wave::Saw, *midi, env, amplitude)
                    .on(Bus::Duck)
                    .pan(pan + lean)
                    .detune(detune)
                    .reverb(1.)
                    .filter(FilterSpec::lowpass(cutoff, cutoff, 0.7)),
            );
        }
    }
}

/// A single bright line, on the dry bus and into the echo.
fn lead(voices: &mut Voices, midi: i32, duration: f32, amplitude: f32, events: &mut Events) {
    for pan in [-0.42, 0.42] {
        voices.start(
            VoiceSpec::new(
                Wave::Saw,
                midi,
                Env::Decay {
                    attack: 0.012,
                    total: duration,
                },
                amplitude,
            )
            .pan(pan)
            .delay(1.)
            .filter(FilterSpec::lowpass(2800., 2800., 0.7)),
        );
    }
    events.notes += 1;
    events.midi = midi;
}

/// A plucked note: sine or triangle, fast attack, exponential tail, a splash of room.
#[allow(clippy::too_many_arguments)]
fn pluck(
    voices: &mut Voices,
    midi: i32,
    amplitude: f32,
    duration: f32,
    wave: Wave,
    pan: f32,
    offset: f32,
    events: &mut Events,
) {
    voices.start(
        VoiceSpec::new(
            wave,
            midi,
            Env::Decay {
                attack: 0.01,
                total: duration,
            },
            amplitude,
        )
        .pan(pan)
        .delayed(offset)
        .reverb(1.),
    );
    events.notes += 1;
    events.midi = midi;
    events.pan = pan;
}

/// The lo-fi keys: a sine plus its octave, through a lowpass, into the room.
fn rhodes(voices: &mut Voices, midi: i32, amplitude: f32, duration: f32, detune: f32, offset: f32) {
    let pan = pitch_class_offset(midi) * 0.55;
    voices.start(
        VoiceSpec::new(
            Wave::Sine,
            midi,
            Env::Decay {
                attack: 0.006,
                total: duration,
            },
            amplitude,
        )
        .pan(pan)
        .detune(detune)
        .delayed(offset)
        .reverb(1.)
        .filter(FilterSpec::lowpass(2000., 2000., 0.7)),
    );
    // The octave gives the Rhodes its bell.
    voices.start(
        VoiceSpec::new(
            Wave::Sine,
            midi + 12,
            Env::Decay {
                attack: 0.006,
                total: duration * 0.6,
            },
            amplitude * 0.28,
        )
        .pan(pan)
        .delayed(offset)
        .reverb(1.),
    );
}

/// A filtered noise hit: hats, claps, the riser, the lo-fi crackle.
fn noise_hit(
    voices: &mut Voices,
    sequence: &mut Sequence,
    filter: FilterSpec,
    pan: f32,
    amplitude: f32,
    duration: f32,
    offset: f32,
) {
    let seed = (sequence.rng.next_f32() * u32::MAX as f32) as u32 | 1;
    voices.start(
        VoiceSpec::new(
            Wave::Noise,
            60,
            Env::Decay {
                attack: 0.001,
                total: duration,
            },
            amplitude,
        )
        .pan(pan)
        .delayed(offset)
        .filter(filter)
        .noise(seed),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundscape::music::Layers;
    use crate::soundscape::synth::MAX_VOICES;

    fn arrangement(preset: Preset) -> Arrangement {
        let mut arrangement = Arrangement::new();
        arrangement.preset = preset;
        if let Some(scape) = preset.scape() {
            arrangement.params = scape.defaults;
        }
        arrangement.energy = 1.0;
        arrangement
    }

    /// Every preset, every step of eight bars: the material must exist, stay in the pool and
    /// never ask for more voices than a device would have to mix.
    #[test]
    fn every_preset_plays_for_eight_bars_without_running_out_of_voices() {
        for preset in [
            Preset::Edm,
            Preset::Ambient,
            Preset::Lofi,
            Preset::Focus,
            Preset::Relax,
            Preset::Sleep,
        ] {
            let mut arrangement = arrangement(preset);
            let mut sequence = Sequence::new(1234);
            let mut voices = Voices::new(48_000.);
            let mut events = Events::default();
            let mut sounded = 0;
            for step in 0..(16 * 8) {
                events.clear();
                play_step(
                    &mut arrangement,
                    &mut sequence,
                    &mut voices,
                    step,
                    &mut events,
                );
                sounded += events.kicks + events.notes + events.chords + events.basses;
                assert!(
                    voices.active_count() <= MAX_VOICES,
                    "{} at step {step}",
                    preset.key()
                );
            }
            assert!(sounded > 0, "{} played nothing", preset.key());
        }
    }

    /// A silenced layer stops producing notes, which is the only thing a layer slider can mean.
    #[test]
    fn a_muted_layer_plays_nothing() {
        let mut arrangement = arrangement(Preset::Edm);
        arrangement.layers = Layers {
            drums: 0.,
            bass: 0.,
            pads: 0.,
            melody: 0.,
            texture: 0.,
        };
        let mut sequence = Sequence::new(7);
        let mut voices = Voices::new(48_000.);
        let mut events = Events::default();
        for step in 0..64 {
            events.clear();
            play_step(
                &mut arrangement,
                &mut sequence,
                &mut voices,
                step,
                &mut events,
            );
            assert!(events.is_empty(), "step {step} played {events:?}");
        }
    }

    /// The same seed has to replay the same material, or the conductor's seed means nothing.
    #[test]
    fn the_same_seed_replays_the_same_notes() {
        let run = |seed: u32| {
            let mut arrangement = arrangement(Preset::Focus);
            let mut sequence = Sequence::new(seed);
            let mut voices = Voices::new(48_000.);
            let mut events = Events::default();
            let mut notes = Vec::new();
            for step in 0..64 {
                events.clear();
                play_step(
                    &mut arrangement,
                    &mut sequence,
                    &mut voices,
                    step,
                    &mut events,
                );
                if events.notes > 0 {
                    notes.push((step, events.midi));
                }
            }
            notes
        };
        assert_eq!(run(99), run(99));
        assert_ne!(run(99), run(100));
    }

    /// A scape holds its degree across the first change of a new scene, so a preset change
    /// hands over on shared notes instead of jumping.
    #[test]
    fn a_preset_change_holds_the_degree_for_one_chord() {
        let mut arrangement = arrangement(Preset::Focus);
        let mut sequence = Sequence::new(5);
        let mut voices = Voices::new(48_000.);
        let mut events = Events::default();
        play_step(&mut arrangement, &mut sequence, &mut voices, 0, &mut events);
        let before = sequence.degree;

        arrangement.preset = Preset::Relax;
        arrangement.params = Preset::Relax.scape().expect("relax").defaults;
        sequence.hold_degree = true;
        play_step(
            &mut arrangement,
            &mut sequence,
            &mut voices,
            32,
            &mut events,
        );
        assert_eq!(sequence.degree, before, "the degree moved on the handover");
        assert!(sequence.chord.is_some());
    }

    /// Lo-fi's second sixteenth has to arrive late: the preset is a limp, not a straight grid.
    #[test]
    fn lofi_swings_the_second_sixteenth() {
        let mut arrangement = arrangement(Preset::Lofi);
        arrangement.layers = Layers {
            drums: 1.,
            bass: 1.,
            pads: 1.,
            melody: 1.,
            texture: 1.,
        };
        let mut sequence = Sequence::new(3);
        let mut voices = Voices::new(48_000.);
        let mut events = Events::default();
        // Step 2 is the swung sixteenth; it starts a hat at a level loud enough to find.
        play_step(&mut arrangement, &mut sequence, &mut voices, 2, &mut events);
        let expected_swing = (arrangement.spb() * 0.14 * 48_000.) as usize;
        let mut first_sound = None;
        for index in 0..14_400 {
            let (mut dry, mut duck) = ([0f32; 2], [0f32; 2]);
            let (mut rev, mut delay) = (0., 0.);
            voices.add_sample(&mut dry, &mut duck, &mut rev, &mut delay);
            if (dry[0].abs() + duck[0].abs()) > 1e-4 && first_sound.is_none() {
                first_sound = Some(index);
            }
        }
        let first_sound = first_sound.expect("the swung hat played");
        assert!(
            first_sound >= expected_swing,
            "the hat arrived at {first_sound}, before the swing of {expected_swing}"
        );
    }
}
