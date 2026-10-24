//! The engine: what the audio callback runs, and the only place a sample is produced.
//!
//! One sample clock owns everything. The browser polls every 25 ms and schedules events up to
//! 180 ms ahead, because it cannot know when a callback will happen; here the callback *is* the
//! clock, so a sixteenth lands on the sample its tempo says it lands on and a swung note is a
//! delay of a known number of samples rather than a jitter of up to one poll.
//!
//! Nothing in `render` allocates, locks or logs. Every buffer is sized when the engine is built,
//! every voice is a slot in an array, and parameters arrive through atomics read once per block.

use super::analyser::Analyser;
use super::control::{Control, Visual, BANDS};
use super::fx::{Duck, Echo, Master, Room};
use super::music::{Arrangement, LayerName, ParamName, Preset};
use super::steps::{play_step, Events, Sequence};
use super::synth::Voices;

/// Sixteenths per bar. Everything here counts steps of this size.
const STEPS_PER_BAR: u64 = 16;

/// A filter and a gain for one of the two noise beds. The beds are what makes a scape feel like
/// a room rather than a chord: a vinyl hiss on lo-fi, and a wide filtered wash under everything
/// else.
///
/// The highpass is a proper DC blocker, `y[n] = a * (y[n-1] + x[n] - x[n-1])`. It has to be: an
/// earlier version integrated the lowpassed noise without subtracting the previous input, which at
/// a corner of 0 Hz left `a = 1` and made the filter a pure accumulator. The bed then walked off
/// towards full scale -- a +0.27 DC offset on every scape, which the compressor and limiter turned
/// into the pumping, crackling sound that was reported as "buggy".
struct Bed {
    noise: u32,
    /// Two cascaded one-pole lowpasses: a single pole leaves the top octave bright enough to hiss.
    lowpass: [f32; 2],
    lowpass_coefficient: f32,
    highpass: f32,
    highpass_coefficient: f32,
    /// The previous lowpassed input, which the DC blocker needs and the old version did not keep.
    previous: f32,
    gain: f32,
}

impl Bed {
    fn new(seed: u32, sample_rate: f32) -> Self {
        let mut bed = Self {
            noise: seed | 1,
            lowpass: [0.; 2],
            lowpass_coefficient: 0.,
            highpass: 0.,
            highpass_coefficient: 0.,
            previous: 0.,
            gain: 0.,
        };
        bed.set_filters(900., 0., sample_rate);
        bed
    }

    fn set_filters(&mut self, lowpass_hz: f32, highpass_hz: f32, sample_rate: f32) {
        self.lowpass_coefficient = 1.
            - (-std::f32::consts::TAU * lowpass_hz.clamp(20., sample_rate * 0.45) / sample_rate)
                .exp();
        self.highpass_coefficient =
            (-std::f32::consts::TAU * highpass_hz.clamp(0., sample_rate * 0.45) / sample_rate)
                .exp();
    }

    #[inline]
    fn sample(&mut self) -> f32 {
        let mut x = self.noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x;
        let white = (x as f32 / u32::MAX as f32) * 2. - 1.;
        for pole in self.lowpass.iter_mut() {
            *pole += self.lowpass_coefficient * (white - *pole);
        }
        let filtered = self.lowpass[1];
        self.highpass = self.highpass_coefficient * (self.highpass + filtered - self.previous);
        self.previous = filtered;
        self.highpass * self.gain
    }
}

/// The low shelf's corner and depth: 0.3 of the signal under 160 Hz is about -3 dB there.
const LOW_SHELF_HZ: f32 = 160.;
const LOW_SHELF_CUT: f32 = 0.3;

fn low_shelf_coefficient(sample_rate: f32) -> f32 {
    1. - (-std::f32::consts::TAU * LOW_SHELF_HZ / sample_rate).exp()
}

pub struct Engine {
    sample_rate: f32,
    voices: Voices,
    sequence: Sequence,
    arrangement: Arrangement,
    room: Room,
    echo: Echo,
    duck: Duck,
    master: Master,
    vinyl: Bed,
    texture: Bed,
    analyser: Analyser,
    /// The global step counter. It never resets, so a preset change can restart its own grid
    /// (`step - step_base`) while the bar the change waits for keeps running.
    step: u64,
    /// The step the current preset started on.
    step_base: u64,
    /// Samples until the next sixteenth fires.
    until_step: u32,
    /// Samples since the preset started, for the arc and the drift.
    elapsed_samples: u64,
    /// A preset waiting for the next bar. Readable so a surface can show it.
    pending_preset: Option<Preset>,
    /// A transport gate: 20 ms up, 40 ms down, so pause is not a click.
    gate: f32,
    gate_target: f32,
    gate_step_up: f32,
    gate_step_down: f32,
    /// How often the spectrum is published.
    report_every: u32,
    report_count: u32,
    events: Events,
    /// Set when the bed filters need re-deriving because a parameter moved.
    beds_dirty: bool,
    /// A DC blocker on the reverb send. The room is fed everything above the sub: a six-second
    /// tail on a 60 Hz sub is mud, and it is the one thing that makes a large `space` unlistenable.
    reverb_highpass: f32,
    reverb_highpass_coefficient: f32,
    /// The last values written to the room, so a parameter that has not moved does not rebuild
    /// the comb network.
    room_space: f32,
    room_brightness: f32,
    /// The room's return gain, one-poled towards the preset's own.
    ///
    /// `reverb_send()` is a property of the preset and the scene's `space`, so it steps at a preset
    /// change: 0.16 for EDM to 0.55 for Ambient is a gain of 3.4 on a tail that is still ringing,
    /// which is a click in the reverb rather than a change of room.
    reverb_gain: f32,
    reverb_gain_step: f32,
    /// A low shelf on the sidechained bus, which carries the pads and the bass. After the
    /// analyser's headroom fix the loudest bands were 100-200 Hz on every preset, and the pads'
    /// detuned saws are most of that: the shelf takes `LOW_SHELF_CUT` of everything under the
    /// corner out of that bus and leaves the kick and the sub, on the dry bus, alone.
    duck_low: [f32; 2],
    duck_low_coefficient: f32,
    echo_delay: f32,
    /// A new preset's echo time is reached quickly; a tempo that drifts glides.
    echo_snap: bool,
    seed: u32,
    reported_preset: Option<Preset>,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        let mut engine = Self {
            sample_rate,
            voices: Voices::new(sample_rate),
            sequence: Sequence::new(0),
            arrangement: Arrangement::new(),
            room: Room::new(sample_rate),
            echo: Echo::new(sample_rate),
            duck: Duck::new(sample_rate),
            master: Master::new(sample_rate),
            vinyl: Bed::new(0x1b56_c4e9, sample_rate),
            texture: Bed::new(0x6a09_e667, sample_rate),
            analyser: Analyser::new(sample_rate),
            step: 0,
            step_base: 0,
            until_step: 0,
            elapsed_samples: 0,
            pending_preset: None,
            gate: 0.,
            gate_target: 0.,
            gate_step_up: 1. / (0.02 * sample_rate),
            gate_step_down: 1. / (0.04 * sample_rate),
            report_every: Analyser::frames_per_update(sample_rate) as u32,
            report_count: 0,
            events: Events::default(),
            beds_dirty: true,
            reverb_highpass: 0.,
            reverb_highpass_coefficient: 1. - (-std::f32::consts::TAU * 180. / sample_rate).exp(),
            room_space: -1.,
            room_brightness: -1.,
            reverb_gain: 0.4,
            reverb_gain_step: 1. - (-1. / (0.08 * sample_rate)).exp(),
            duck_low: [0.; 2],
            duck_low_coefficient: low_shelf_coefficient(sample_rate),
            echo_delay: -1.,
            echo_snap: true,
            seed: 0,
            reported_preset: None,
        };
        engine.arrangement.preset = Preset::Edm;
        engine.apply_preset(Preset::Edm);
        engine
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn preset(&self) -> Preset {
        self.arrangement.preset
    }

    pub fn pending_preset(&self) -> Option<Preset> {
        self.pending_preset
    }

    pub fn step(&self) -> u64 {
        self.step
    }

    /// The device changed its rate between runs, or a headset was plugged into one that did.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.voices.set_sample_rate(sample_rate);
        self.room.set_sample_rate(sample_rate);
        self.echo.set_sample_rate(sample_rate);
        self.duck.set_sample_rate(sample_rate);
        self.master.set_sample_rate(sample_rate);
        self.analyser.set_sample_rate(sample_rate);
        self.gate_step_up = 1. / (0.02 * sample_rate);
        self.gate_step_down = 1. / (0.04 * sample_rate);
        self.report_every = Analyser::frames_per_update(sample_rate) as u32;
        self.reverb_highpass_coefficient = 1. - (-std::f32::consts::TAU * 180. / sample_rate).exp();
        self.duck_low_coefficient = low_shelf_coefficient(sample_rate);
        self.beds_dirty = true;
    }

    /// Silence everything and keep rendering zeros. Used when the window hands the device back.
    pub fn silence(&mut self) {
        self.voices.silence();
        self.room.clear();
        self.echo.clear();
        self.gate = 0.;
        self.duck = Duck::new(self.sample_rate);
    }

    /// Everything the app asked for, read once per block.
    fn apply_control(&mut self, control: &Control) {
        self.arrangement.params = control.params();
        self.arrangement.layers = control.layers();
        self.arrangement.energy = control.energy();
        self.arrangement.adapt = control.adapt();
        self.arrangement.activity = control.activity();
        self.master.set_volume(control.volume());

        if control.seed() != self.seed {
            self.seed = control.seed();
            self.sequence.set_seed(self.seed);
        }

        let wanted = control.preset();
        if wanted != self.arrangement.preset {
            if !control.playing() {
                self.apply_preset(wanted);
            } else {
                // A preset change waits for the next bar so it does not cut a held chord in
                // half; asking for it a second time takes it immediately, which is the escape
                // hatch from waiting.
                if self.pending_preset == Some(wanted) {
                    self.apply_preset(wanted);
                } else {
                    self.pending_preset = Some(wanted);
                }
            }
        }

        self.gate_target = if control.playing() { 1. } else { 0. };
        if control.playing() && self.until_step == 0 && self.step == 0 {
            self.until_step = 0;
        }
    }

    /// Move to a preset now. The grid keeps running; the new preset hears it from its own zero.
    fn apply_preset(&mut self, preset: Preset) {
        let outgoing = self.arrangement.preset;
        // A scape hands over on notes it shares with the outgoing chord instead of jumping.
        if let (Some(to), Some(_from)) = (preset.scape(), outgoing.scape()) {
            if let (Some(chord), Some(_)) = (self.sequence.chord, Some(to)) {
                let mut probe = self.arrangement.clone();
                probe.preset = preset;
                self.sequence.degree = probe.nearest_degree(chord);
                self.sequence.hold_degree = true;
            }
        }
        self.arrangement.preset = preset;
        self.sequence.reset_grid();
        self.pending_preset = None;
        self.step_base = self.step;
        self.elapsed_samples = 0;
        self.beds_dirty = true;
        self.room_space = -1.;
        self.echo_snap = true;
        self.master.set_trim(preset.trim());
        let (_, feedback, wet) = preset.echo();
        self.echo.set_character(feedback, wet);
    }

    /// The tempo, in samples per sixteenth.
    fn step_samples(&self) -> u32 {
        let seconds = self.arrangement.spb() / 4.;
        (seconds * self.sample_rate).max(1.) as u32
    }

    /// Render interleaved frames into `out`, which the device hands over as its own buffer.
    pub fn render(&mut self, out: &mut [f32], channels: usize, control: &Control, visual: &Visual) {
        self.apply_control(control);
        if self.master.volume() != control.volume() {
            self.master.set_volume(control.volume());
        }
        if self.beds_dirty {
            self.update_beds();
        }

        let frames = out.len() / channels;
        for frame in 0..frames {
            // The gate: a paused transport fades out instead of stopping dead, and a faded-out
            // engine retires its voices rather than leaving them to come back mid-note.
            if self.gate < self.gate_target {
                self.gate = (self.gate + self.gate_step_up).min(self.gate_target);
            } else if self.gate > self.gate_target {
                self.gate = (self.gate - self.gate_step_down).max(self.gate_target);
            }
            if self.gate == 0. && self.gate_target == 0. {
                self.voices.silence();
            }

            if self.until_step == 0 && self.gate_target > 0. {
                self.fire_step();
                self.until_step = self.step_samples();
            }

            let mut dry = [0f32; 2];
            let mut duck = [0f32; 2];
            let mut reverb_send = 0f32;
            let mut echo_send = 0f32;
            if self.gate > 0. {
                self.voices
                    .add_sample(&mut dry, &mut duck, &mut reverb_send, &mut echo_send);

                // The beds sit under everything: a vinyl hiss on lo-fi, a filtered wash under the
                // scapes. Both are the browser's two buffer sources.
                let beds = self.vinyl.sample() + self.texture.sample();
                dry[0] += beds;
                dry[1] += beds;
                reverb_send += beds * 0.3;
            }

            for (channel, low) in self.duck_low.iter_mut().enumerate() {
                *low += self.duck_low_coefficient * (duck[channel] - *low);
                duck[channel] -= LOW_SHELF_CUT * *low;
            }
            let ducked = self.duck.apply(duck);
            // One-pole highpass on the send: the room hears the mid and top, not the sub.
            self.reverb_highpass +=
                self.reverb_highpass_coefficient * (reverb_send - self.reverb_highpass);
            let room_input = reverb_send - self.reverb_highpass;
            let room = self.room.process([room_input, room_input]);
            let reverb_target = self.arrangement.reverb_send();
            self.reverb_gain += self.reverb_gain_step * (reverb_target - self.reverb_gain);
            let echo = self.echo.process([echo_send, echo_send]);

            // The gate is applied once, at the output. Putting it on the dry bus as well made
            // pause a squared fade on the notes and a linear one on the room, so the reverb rose
            // relative to the mix on every pause.
            let mix = [
                dry[0] + ducked[0] + room[0] * self.reverb_gain + echo[0],
                dry[1] + ducked[1] + room[1] * self.reverb_gain + echo[1],
            ];
            let level = self.master.process(mix);

            self.analyser.push((level[0] + level[1]) * 0.5);

            let output = [
                (level[0] * self.gate).clamp(-1., 1.),
                (level[1] * self.gate).clamp(-1., 1.),
            ];
            for channel in 0..channels {
                out[frame * channels + channel] = output[channel.min(1)];
            }

            if self.gate_target > 0. {
                self.elapsed_samples += 1;
                self.arrangement.elapsed = self.elapsed_samples as f32 / self.sample_rate;
                if self.until_step > 0 {
                    self.until_step -= 1;
                }
            }

            self.report_count += 1;
            if self.report_count >= self.report_every {
                self.report_count = 0;
                self.publish(visual);
            }
        }
        // Once per block is enough for the state: a preset change is not a per-frame event.
        if self.reported_preset != Some(self.arrangement.preset) || self.gate_target == 0. {
            self.reported_preset = Some(self.arrangement.preset);
            self.publish_state(visual);
        }
    }

    fn fire_step(&mut self) {
        // A pending preset takes its turn on the bar line.
        if let Some(pending) = self.pending_preset {
            if (self.step - self.step_base).is_multiple_of(STEPS_PER_BAR) {
                self.apply_preset(pending);
            }
        }
        let local = (self.step - self.step_base) as u32;
        self.events.clear();
        let mut played = Events::default();
        play_step(
            &mut self.arrangement,
            &mut self.sequence,
            &mut self.voices,
            local,
            &mut played,
        );
        self.events.absorb(&played);

        // A kick ducks the pads and the bass, which is the whole point of the arrangement.
        if played.kicks > 0 {
            self.duck.trigger();
        }

        // The room follows `space` and the echo follows the tempo, once per step: cheap, and
        // frequent enough that a dragged slider is heard within a sixteenth.
        self.tune_effects();
        self.step += 1;
    }

    fn tune_effects(&mut self) {
        let space = self.arrangement.room_target();
        let brightness = self.arrangement.eff(ParamName::Brightness);
        if (space - self.room_space).abs() > 0.02
            || (brightness - self.room_brightness).abs() > 0.02
        {
            self.room.set_space(space, brightness);
            self.room_space = space;
            self.room_brightness = brightness;
        }
        let delay = self.arrangement.spb() * self.arrangement.preset.echo().0;
        if self.echo_snap {
            self.echo.retune_delay_seconds(delay);
            self.echo_delay = delay;
            self.echo_snap = false;
        } else if (delay - self.echo_delay).abs() > 0.005 {
            self.echo.set_delay_seconds(delay);
            self.echo_delay = delay;
        }
    }

    fn update_beds(&mut self) {
        self.beds_dirty = false;
        let brightness = self.arrangement.eff(ParamName::Brightness);
        let texture_level = if self.arrangement.is_scape() {
            self.arrangement.layers.get(LayerName::Texture)
        } else {
            0.
        };
        self.texture.gain = (0.012 + 0.022 * self.arrangement.eff(ParamName::Texture))
            * texture_level
            * self.arrangement.preset.air();
        self.texture
            .set_filters(250. + 2800. * brightness, 0., self.sample_rate);
        self.vinyl.gain = if self.arrangement.preset == Preset::Lofi {
            0.012 * self.arrangement.layers.get(LayerName::Texture)
        } else {
            0.
        };
        self.vinyl.set_filters(4200., 600., self.sample_rate);
    }

    fn publish(&mut self, visual: &Visual) {
        let (rms, peak) = self.analyser.take_level();
        visual.write_spectrum(self.analyser.bands(), rms, peak);
        if !self.events.is_empty() {
            visual.write_events(
                self.events.kicks,
                self.events.notes + self.events.chords,
                self.events.midi,
                self.events.pan,
            );
            self.events.clear();
        }
        self.publish_state(visual);
    }

    fn publish_state(&self, visual: &Visual) {
        let phase = if self.arrangement.is_scape() {
            self.arrangement.arc().phase
        } else {
            super::music::ArcPhase::Bloom
        };
        visual.write_state(
            self.arrangement.preset,
            phase,
            self.arrangement.energy,
            self.voices.active_count() as u32,
            (self.step - self.step_base) as u32,
        );
    }

    /// The bands, for a caller that wants them without a `Visual` (tests, the probe binary).
    pub fn bands(&self) -> [f32; BANDS] {
        self.analyser.bands()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device's worth of frames, rendered the way the callback does it.
    fn render(engine: &mut Engine, control: &Control, visual: &Visual, frames: usize) -> Vec<f32> {
        let mut buffer = vec![0f32; frames * 2];
        engine.render(&mut buffer, 2, control, visual);
        buffer
    }

    fn energy(buffer: &[f32]) -> f32 {
        buffer.iter().map(|sample| sample.abs()).sum::<f32>() / buffer.len() as f32
    }

    #[test]
    fn a_stopped_engine_produces_silence() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_playing(false);
        let buffer = render(&mut engine, &control, &visual, 24_000);
        assert_eq!(energy(&buffer), 0.);
    }

    /// Pressing play has to make sound, and it has to be the mix rather than one note.
    #[test]
    fn playing_produces_a_signal_in_range() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Edm);
        control.set_volume(0.8);
        control.set_playing(true);
        // Three seconds: the first bar plus enough for the phrase to fill out.
        let buffer = render(&mut engine, &control, &visual, 48_000 * 3);
        let level = energy(&buffer);
        assert!(level > 0.001, "the mix is silent, level {level}");
        assert!(
            buffer.iter().all(|sample| sample.abs() <= 1.0),
            "the output clipped past full scale"
        );
        // The output is not a constant, which would mean a stuck DC offset.
        let first = buffer[24_000];
        let last = buffer[buffer.len() - 1];
        assert!((first - last).abs() > 1e-6, "the engine output is constant");
    }

    /// The largest jump between two consecutive samples of one channel, which is what a click is.
    ///
    /// Every layer here is a shaped signal: envelopes are continuous, gains are smoothed and the
    /// room is a filter. A step in any of them shows up as one sample delta far above what the
    /// material itself asks for, so this is the measurement that says whether a transition is
    /// clean rather than a matter of taste.
    fn worst_step(buffer: &[f32]) -> f32 {
        buffer
            .chunks(2)
            .map(|frame| frame[0])
            .collect::<Vec<f32>>()
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0f32, f32::max)
    }

    /// The spectrum has to keep headroom, because the field is drawn from it.
    ///
    /// A saturated analyser is not a visual that looks loud, it is a visual that stops moving: every
    /// ridge sits at its own maximum and the picture no longer answers the music. This is the
    /// measurement that caught it (a third of every band-sample pinned at 1.00) and the one that
    /// keeps it from coming back.
    #[test]
    fn the_spectrum_keeps_headroom_across_the_presets() {
        for preset in [Preset::Edm, Preset::Ambient, Preset::Lofi, Preset::Focus] {
            let control = Control::new();
            let visual = Visual::new();
            let mut engine = Engine::new(48_000.);
            control.set_preset(preset);
            control.set_volume(0.8);
            control.set_playing(true);
            render(&mut engine, &control, &visual, 48_000);
            let mut all: Vec<f32> = Vec::new();
            for _ in 0..80 {
                render(&mut engine, &control, &visual, 480);
                all.extend(engine.bands().iter().copied());
            }
            all.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let at = |p: f32| all[((all.len() - 1) as f32 * p) as usize];
            let pinned = all.iter().filter(|band| **band > 0.98).count() * 100 / all.len();
            let name = preset.key();
            assert!(
                at(0.90) < 0.85,
                "{name}: the ninetieth percentile is {} — the spectrum is saturated",
                at(0.90)
            );
            assert_eq!(
                pinned, 0,
                "{name}: {pinned}% of band-samples are pinned at the top"
            );
            assert!(
                at(0.50) > 0.05,
                "{name}: the median is {} — nothing is visible",
                at(0.50)
            );
        }
    }

    /// A preset change is the moment the operator is listening hardest, and it is where the echo
    /// used to jump its read position and the room return used to step its gain.
    ///
    /// Measured rather than asserted by ear: the largest move between consecutive samples across
    /// the change, against the peak of the same seconds. The materials themselves are band-limited
    /// (a saw's own edge is the largest honest step), so the bound is relative, not absolute.
    #[test]
    fn a_preset_change_does_not_step_the_mix() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Ambient);
        control.set_volume(0.8);
        control.set_playing(true);
        // Two seconds of an ambient tail, so there is something ringing to step under.
        render(&mut engine, &control, &visual, 96_000);

        control.set_preset(Preset::Edm);
        let mut worst = 0f32;
        let mut peak = 0f32;
        let mut changing = 0f32;
        for block in 0..40 {
            let buffer = render(&mut engine, &control, &visual, 4_800);
            if block < 6 {
                // The bar line is within the first two blocks at 124 bpm; take the change itself.
                changing = changing.max(worst_step(&buffer));
            }
            worst = worst.max(worst_step(&buffer));
            peak = peak.max(buffer.iter().fold(0f32, |a, b| a.max(b.abs())));
        }
        assert!(peak > 0.02, "nothing sounded, peak {peak}");
        assert!(
            changing < peak * 0.2,
            "the preset change stepped the mix by {changing} at peak {peak}"
        );
    }

    /// Pausing fades rather than cutting, then goes properly quiet.
    #[test]
    fn pausing_fades_out_and_stays_out() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Ambient);
        control.set_playing(true);
        render(&mut engine, &control, &visual, 48_000 * 2);

        control.set_playing(false);
        // The first frames are the tail of the fade, not a hard cut.
        let fade = render(&mut engine, &control, &visual, 480);
        assert!(
            energy(&fade) > 0.,
            "the stack cut the sound instead of fading"
        );
        render(&mut engine, &control, &visual, 48_000);
        let silence = render(&mut engine, &control, &visual, 4_800);
        assert_eq!(energy(&silence), 0., "a paused engine kept sounding");
    }

    /// The arc of an hour has to be reachable from a test, so the transport runs a whole scape
    /// session and reports the phase it ends on.
    #[test]
    fn a_scape_reports_its_arc_phase() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Focus);
        control.set_playing(true);
        render(&mut engine, &control, &visual, 24_000);
        assert_eq!(visual.preset(), Preset::Focus);
        assert_eq!(visual.phase(), super::super::music::ArcPhase::Bloom);
        assert!(visual.voices() > 0, "a scape should hold voices");
    }

    /// A preset change while playing waits for the bar; asking twice takes it immediately.
    #[test]
    fn a_preset_change_waits_for_the_bar_unless_asked_twice() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Edm);
        control.set_playing(true);
        // Render into the middle of a bar.
        render(&mut engine, &control, &visual, 24_000);

        control.set_preset(Preset::Lofi);
        render(&mut engine, &control, &visual, 480);
        assert_eq!(engine.pending_preset(), Some(Preset::Lofi));
        assert_eq!(engine.preset(), Preset::Edm);

        // Asking for the same one again lands it now instead of waiting.
        control.set_preset(Preset::Edm);
        control.set_preset(Preset::Lofi);
        render(&mut engine, &control, &visual, 480);
        assert_eq!(engine.preset(), Preset::Lofi);
    }

    /// The visuals have to be told about the music, or the window is a screensaver.
    #[test]
    fn the_visual_report_follows_the_music() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Edm);
        control.set_playing(true);
        render(&mut engine, &control, &visual, 48_000 * 2);
        assert!(
            visual.kicks() > 0,
            "no kick was reported in two bars of EDM"
        );
        assert!(visual.rms() > 0., "the level is zero while the mix plays");
        let bands = visual.bands();
        assert!(
            bands.iter().any(|band| *band > 0.01),
            "the spectrum is flat: {bands:?}"
        );
    }

    /// A 96 kHz device is what this Mac actually offers, and the engine has to answer there.
    #[test]
    fn the_engine_runs_at_a_device_rate_of_96k() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(96_000.);
        control.set_preset(Preset::Focus);
        control.set_playing(true);
        let buffer = render(&mut engine, &control, &visual, 96_000 * 2);
        assert!(energy(&buffer) > 0.0005, "silent at 96 kHz");
        assert!(buffer.iter().all(|sample| sample.abs() <= 1.0));
    }

    /// Mono devices, surround devices, anything the hardware asks for: the frames are interleaved
    /// and every channel gets the same level.
    #[test]
    fn any_channel_count_is_filled() {
        let control = Control::new();
        let visual = Visual::new();
        for channels in 1..=6 {
            let mut engine = Engine::new(48_000.);
            control.set_preset(Preset::Ambient);
            control.set_playing(true);
            let mut buffer = vec![0f32; 24_000 * channels];
            engine.render(&mut buffer, channels, &control, &visual);
            for frame in 0..24_000 {
                for channel in 0..channels {
                    assert!(
                        buffer[frame * channels + channel].is_finite(),
                        "{channels} channels produced a non-finite sample"
                    );
                }
            }
            assert!(
                buffer.iter().any(|sample| *sample != 0.),
                "{channels} channels produced nothing at all"
            );
        }
    }

    /// The seed is the conductor's: the same seed has to be the same session.
    #[test]
    fn the_same_seed_renders_the_same_buffer() {
        let render_with = |seed: u32| {
            let control = Control::new();
            let visual = Visual::new();
            control.set_seed(seed);
            control.set_preset(Preset::Focus);
            control.set_playing(true);
            let mut engine = Engine::new(48_000.);
            render(&mut engine, &control, &visual, 48_000)
        };
        assert_eq!(render_with(42), render_with(42));
        assert_ne!(render_with(42), render_with(43));
    }

    /// A rate change mid-session must not panic or leave the FX lines the wrong length.
    #[test]
    fn changing_the_device_rate_keeps_the_engine_usable() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(48_000.);
        control.set_preset(Preset::Lofi);
        control.set_playing(true);
        render(&mut engine, &control, &visual, 24_000);
        engine.set_sample_rate(96_000.);
        let buffer = render(&mut engine, &control, &visual, 48_000);
        assert!(energy(&buffer) > 0.0005);
        assert!(buffer.iter().all(|sample| sample.abs() <= 1.0));
    }
}

#[cfg(test)]
mod quality {
    //! What every preset has to measure, on any change to the voices or the buses.
    //!
    //! These are not taste: each one is a failure that was heard before it was measured. The noise
    //! bed's DC blocker was an integrator, so every scape walked to a +0.27 offset and the
    //! compressor turned it into pumping, crackling sound; a level that drifts over half a minute
    //! is that class of bug, and a DC offset is its signature.

    use super::*;

    const RATE: f32 = 48_000.;
    const PRESETS: [Preset; 6] = [
        Preset::Edm,
        Preset::Ambient,
        Preset::Lofi,
        Preset::Focus,
        Preset::Relax,
        Preset::Sleep,
    ];

    struct Reading {
        rms: f32,
        peak: f32,
        dc: f32,
    }

    fn read(buffer: &[f32]) -> Reading {
        let frames = buffer.len() / 2;
        let mut peak: f32 = 0.;
        let mut sum = 0f64;
        let mut dc = 0f64;
        for frame in 0..frames {
            let value = buffer[frame * 2];
            assert!(value.is_finite(), "a non-finite sample reached the device");
            peak = peak.max(value.abs());
            sum += (value as f64) * (value as f64);
            dc += value as f64;
        }
        Reading {
            rms: (sum / frames as f64).sqrt() as f32,
            peak,
            dc: (dc / frames as f64) as f32,
        }
    }

    fn render(engine: &mut Engine, control: &Control, visual: &Visual, frames: usize) -> Vec<f32> {
        let mut buffer = vec![0f32; frames * 2];
        engine.render(&mut buffer, 2, control, visual);
        buffer
    }

    /// Half a minute per preset: no offset, no clipping, and a level that is where it was.
    ///
    /// Ignored by default: 28 s of audio per preset at two volumes is 20.8 s of an unoptimized
    /// test run, where the other 288 core tests finish in under 2 s (measured 2026-10-24). Run it
    /// before a soundscape change with
    /// `cargo test -p sjel-gpui-core --release -- --ignored every_preset_stays_centred_and_steady`.
    #[test]
    #[ignore = "slow in debug; run with --release --ignored before a soundscape change"]
    fn every_preset_stays_centred_and_steady() {
        for preset in PRESETS {
            for volume in [0.65f32, 1.0] {
                let control = Control::new();
                let visual = Visual::new();
                let mut engine = Engine::new(RATE);
                control.set_preset(preset);
                control.set_volume(volume);
                control.set_playing(true);

                let opening = read(&render(
                    &mut engine,
                    &control,
                    &visual,
                    (RATE * 4.) as usize,
                ));
                let closing = read(&render(
                    &mut engine,
                    &control,
                    &visual,
                    (RATE * 20.) as usize,
                ));
                let tail = read(&render(
                    &mut engine,
                    &control,
                    &visual,
                    (RATE * 4.) as usize,
                ));

                for (name, reading) in [
                    ("opening", &opening),
                    ("closing", &closing),
                    ("tail", &tail),
                ] {
                    assert!(
                        reading.dc.abs() < 0.005,
                        "{} at volume {volume}: {name} sits at a DC offset of {:+.4}",
                        preset.label(),
                        reading.dc
                    );
                    assert!(
                        reading.peak <= 1.0,
                        "{} at volume {volume}: {name} peaked at {}",
                        preset.label(),
                        reading.peak
                    );
                    assert!(
                        (0.005..=0.3).contains(&reading.rms),
                        "{} at volume {volume}: {name} measures {} RMS",
                        preset.label(),
                        reading.rms
                    );
                }
                // A runaway bed grows without bound; a real arrangement breathes. Four times either
                // way is generous, and a drifting filter blows past it.
                let ratio = closing.rms / opening.rms.max(1e-6);
                assert!(
                    (0.25..=4.).contains(&ratio),
                    "{} at volume {volume}: the level drifted by {ratio}x over half a minute",
                    preset.label()
                );
            }
        }
    }

    /// Twenty seconds of each preset at volume 0.8, after a four-second settle: rms, peak, and the
    /// median of each eighth of the band profile. The mix pass is measured with this, not by ear:
    /// `cargo test -p sjel-gpui-core --lib mix_report -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn mix_report() {
        for preset in PRESETS {
            let control = Control::new();
            let visual = Visual::new();
            let mut engine = Engine::new(RATE);
            control.set_preset(preset);
            control.set_volume(0.8);
            control.set_playing(true);
            render(&mut engine, &control, &visual, (RATE * 4.) as usize);
            let mut buffer = Vec::new();
            let mut profile: Vec<Vec<f32>> = vec![Vec::new(); 8];
            for _ in 0..400 {
                buffer.extend(render(&mut engine, &control, &visual, 2_400));
                for (band, value) in engine.bands().iter().enumerate() {
                    profile[band * 8 / BANDS].push(*value);
                }
            }
            let reading = read(&buffer);
            let medians: Vec<String> = profile
                .iter_mut()
                .map(|values| {
                    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    format!("{:.2}", values[values.len() / 2])
                })
                .collect();
            println!(
                "{:8} rms {:.3} peak {:.3}  bands/8 {}",
                preset.key(),
                reading.rms,
                reading.peak,
                medians.join(" ")
            );
        }
    }

    /// Pausing has to leave nothing behind: a bed or a room that keeps ringing is a stuck note.
    #[test]
    fn a_paused_engine_goes_fully_silent() {
        let control = Control::new();
        let visual = Visual::new();
        let mut engine = Engine::new(RATE);
        for preset in PRESETS {
            control.set_preset(preset);
            control.set_playing(true);
            let _ = render(&mut engine, &control, &visual, (RATE * 3.) as usize);
            control.set_playing(false);
            let _ = render(&mut engine, &control, &visual, (RATE * 2.) as usize);
            let silent = read(&render(
                &mut engine,
                &control,
                &visual,
                (RATE * 0.5) as usize,
            ));
            assert_eq!(
                silent.peak,
                0.,
                "{} kept sounding after pause: peak {}",
                preset.label(),
                silent.peak
            );
        }
    }

    /// The echo is retuned as the tempo drifts; a delay that steps clicks, and this is the check
    /// that it does not.
    #[test]
    fn a_retuned_echo_does_not_click() {
        let mut echo = super::super::fx::Echo::new(RATE);
        echo.set_delay_seconds(0.2);
        for _ in 0..(RATE as usize / 10) {
            echo.process([0., 0.]);
        }
        // Feed one impulse, then walk the delay time the way a drifting tempo does.
        let mut previous = 0f32;
        let mut biggest = 0f32;
        // The impulse itself is the first thing through, and it is a step by construction: what this
        // measures is the tail moving, so the walk starts after it.
        let _ = echo.process([1., 1.]);
        for step in 0..(RATE as usize * 2) {
            echo.set_delay_seconds(0.2 + 0.05 * (step as f32 / RATE).sin());
            let output = echo.process([0., 0.]);
            biggest = biggest.max((output[0] - previous).abs());
            previous = output[0];
        }
        assert!(
            biggest < 0.5,
            "the echo stepped by {biggest} while retuning"
        );
        assert!(previous.is_finite());
    }
}
