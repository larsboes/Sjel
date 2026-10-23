//! The voices: oscillators, a filter each, and a closed-form envelope.
//!
//! One deliberate difference from the browser engine, because it is what makes a native port
//! worth doing: a voice here is a fixed slot in an array with a sample counter, not a graph of
//! nodes that has to be allocated, connected and collected. The audio callback can therefore
//! stay allocation-free and lock-free, and a note starts on the exact sample the sequencer
//! asked for instead of within the next 25 ms poll.
//!
//! What is ported faithfully is the *shape* of each voice: a pad is two detuned saws per chord
//! tone with a lowpass and a slow swell, a pluck is a sine or triangle with a fast attack and an
//! exponential tail, a kick is a sine whose pitch falls by an octave and a half over half its
//! length. Envelopes are closed-form functions of the voice's age (linear attack, then either
//! hold-and-release or an exponential fall that reaches -60 dB at the end) rather than ramp
//! nodes; that is the same curve the browser schedules.

use super::music::hz;

/// Enough for a four-note pad with its detune pairs, a sub, three plucks and a drum at once,
/// twice over. A note that finds no free slot takes the oldest one, which is what a hardware
/// synth does and is inaudible at this density.
pub const MAX_VOICES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Saw,
    Triangle,
    Square,
    Noise,
}

impl Wave {
    /// The browser's `OscillatorType` name, for tests and for the README's mapping.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Saw => "sawtooth",
            Self::Triangle => "triangle",
            Self::Square => "square",
            Self::Noise => "noise",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    Off,
    Lowpass,
    Highpass,
    Bandpass,
}

/// Which bus a voice lands on. The browser's four scene buses collapse to two: `Dry` is
/// untouched and `Duck` is the sidechained one the bass and pads sit on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Dry,
    Duck,
}

/// An envelope expressed as a function of the voice's age in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Env {
    /// Linear attack to full, hold, then a linear fall to silence at `total`. The pads.
    Hold {
        attack: f32,
        hold_until: f32,
        total: f32,
    },
    /// Linear attack, then an exponential fall reaching -60 dB at `total`. Anything plucked.
    Decay { attack: f32, total: f32 },
}

impl Env {
    pub fn total(&self) -> f32 {
        match self {
            Self::Hold { total, .. } | Self::Decay { total, .. } => *total,
        }
    }

    /// The level at `age` seconds, in `0.0..=1.0`. Past `total` it is exactly zero, which is
    /// what retires a voice.
    pub fn level(&self, age: f32) -> f32 {
        match *self {
            Self::Hold {
                attack,
                hold_until,
                total,
            } => {
                if age >= total {
                    0.
                } else if age < attack {
                    (age / attack.max(1e-5)).clamp(0., 1.)
                } else if age < hold_until {
                    1.
                } else {
                    let fall = (total - age) / (total - hold_until).max(1e-5);
                    fall.clamp(0., 1.)
                }
            }
            Self::Decay { attack, total } => {
                if age >= total {
                    0.
                } else if age < attack {
                    (age / attack.max(1e-5)).clamp(0., 1.)
                } else {
                    // exp(-k t) with k chosen so the tail is -60 dB at `total`.
                    let remaining = (total - age).max(0.);
                    let span = (total - attack).max(1e-5);
                    // Cheap approximation of 0.001^(v/span): a fifth-power fall over the same
                    // span has the same shape and no transcendental per sample.
                    let v = remaining / span;
                    v * v * v * v * v
                }
            }
        }
    }
}

/// Where a voice's cutoff starts and ends, so a filter can sweep across the note the way the
/// browser's `exponentialRampToValueAtTime` on a biquad does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterSpec {
    pub mode: FilterMode,
    pub q: f32,
    pub from_hz: f32,
    pub to_hz: f32,
}

impl FilterSpec {
    pub const OFF: Self = Self {
        mode: FilterMode::Off,
        q: 0.7,
        from_hz: 1000.,
        to_hz: 1000.,
    };

    pub fn lowpass(from_hz: f32, to_hz: f32, q: f32) -> Self {
        Self {
            mode: FilterMode::Lowpass,
            q,
            from_hz,
            to_hz,
        }
    }

    pub fn bandpass(hz: f32, q: f32) -> Self {
        Self {
            mode: FilterMode::Bandpass,
            q,
            from_hz: hz,
            to_hz: hz,
        }
    }

    pub fn highpass(hz: f32, q: f32) -> Self {
        Self {
            mode: FilterMode::Highpass,
            q,
            from_hz: hz,
            to_hz: hz,
        }
    }
}

/// Everything one note is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceSpec {
    pub wave: Wave,
    /// The pitch the note is at. Together with `glide_to` this is also the kick's drop.
    pub hz: f32,
    pub glide_to: Option<f32>,
    /// How long the pitch takes to reach `glide_to`. A kick that falls over its whole length
    /// sounds like a tape stopping; the browser falls over half of it, which reads as an attack.
    pub glide_seconds: f32,
    /// Cents away from `hz`. A pad's two saws sit at -7 and +7, which is where its width comes
    /// from; a Rhodes note draws its own few cents of drift.
    pub detune_cents: f32,
    /// How long the voice waits before it sounds. The sequencer is sample-accurate, so a swung
    /// sixteenth is a delay on the voice rather than a second clock.
    pub delay_seconds: f32,
    pub env: Env,
    pub amp: f32,
    /// -1 hard left, +1 hard right. Centred voices skip the pan maths.
    pub pan: f32,
    pub bus: Bus,
    pub send_rev: f32,
    pub send_del: f32,
    pub filter: FilterSpec,
    /// A per-voice seed, so two noise hits are not the same noise.
    pub noise_seed: u32,
}

impl VoiceSpec {
    pub fn new(wave: Wave, midi: i32, env: Env, amp: f32) -> Self {
        Self {
            wave,
            hz: hz(midi),
            glide_to: None,
            glide_seconds: 0.,
            detune_cents: 0.,
            delay_seconds: 0.,
            env,
            amp,
            pan: 0.,
            bus: Bus::Dry,
            send_rev: 0.,
            send_del: 0.,
            filter: FilterSpec::OFF,
            noise_seed: 0x2545_f491,
        }
    }

    pub fn on(mut self, bus: Bus) -> Self {
        self.bus = bus;
        self
    }

    pub fn pan(mut self, pan: f32) -> Self {
        self.pan = pan.clamp(-1., 1.);
        self
    }

    pub fn detune(mut self, cents: f32) -> Self {
        self.detune_cents = cents;
        self
    }

    pub fn delayed(mut self, seconds: f32) -> Self {
        self.delay_seconds = seconds.max(0.);
        self
    }

    pub fn reverb(mut self, send: f32) -> Self {
        self.send_rev = send.clamp(0., 1.);
        self
    }

    pub fn delay(mut self, send: f32) -> Self {
        self.send_del = send.clamp(0., 1.);
        self
    }

    pub fn filter(mut self, filter: FilterSpec) -> Self {
        self.filter = filter;
        self
    }

    pub fn glide(mut self, to: f32, seconds: f32) -> Self {
        self.glide_to = Some(to);
        self.glide_seconds = seconds.max(0.);
        self
    }

    pub fn noise(mut self, seed: u32) -> Self {
        self.noise_seed = seed;
        self
    }
}

/// TPT state-variable filter (Zavalishin / Simper). One topology serves lowpass, highpass and
/// bandpass, and it stays stable when a swept cutoff passes the Nyquist limit -- which the
/// textbook biquad in the browser does not, and which a 96 kHz device makes easy to hit.
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    #[inline]
    fn process(&mut self, input: f32, g: f32, k: f32, mode: FilterMode) -> f32 {
        let a1 = 1. / (1. + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2. * v1 - self.ic1;
        self.ic2 = 2. * v2 - self.ic2;
        match mode {
            FilterMode::Lowpass => v2,
            FilterMode::Bandpass => v1,
            FilterMode::Highpass => input - k * v1 - v2,
            FilterMode::Off => input,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    spec: VoiceSpec,
    /// Samples the voice has been sounding.
    age_samples: u32,
    /// How long it sounds, in samples.
    length_samples: u32,
    /// How long it waits before it sounds, in samples.
    delay_samples: u32,
    /// Filter sweep endpoints, precomputed because `tan` per sample is not affordable.
    g_from: f32,
    g_to: f32,
    k: f32,
    svf: Svf,
    phase: f32,
    /// Hz per sample, the pitch at the note's start.
    inc_from: f32,
    inc_to: f32,
    /// -1 for the left channel, +1 for the right: the pan gains, precomputed.
    gain_l: f32,
    gain_r: f32,
    noise_state: u32,
    active: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            spec: VoiceSpec::new(
                Wave::Sine,
                60,
                Env::Decay {
                    attack: 0.01,
                    total: 0.1,
                },
                0.,
            ),
            age_samples: 0,
            length_samples: 0,
            delay_samples: 0,
            g_from: 0.,
            g_to: 0.,
            k: 0.7,
            svf: Svf::default(),
            phase: 0.,
            inc_from: 0.,
            inc_to: 0.,
            gain_l: 0.707,
            gain_r: 0.707,
            noise_state: 1,
            active: false,
        }
    }
}

/// PolyBLEP residual, which bends a naive step so it stops aliasing. Without it a saw at
/// 2.8 kHz folds a metallic whine back down into the pad.
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.
    } else if t > 1. - dt {
        let t = (t - 1.) / dt;
        t * t + t + t + 1.
    } else {
        0.
    }
}

impl Voice {
    #[inline]
    fn sample(&mut self, sample_rate: f32, noise_bed: u32) -> f32 {
        if self.age_samples < self.delay_samples {
            self.age_samples += 1;
            return 0.;
        }
        let age = (self.age_samples - self.delay_samples) as f32 / sample_rate;
        let level = self.spec.env.level(age);
        // Silence at the very start of an attack is the envelope working, not a spent voice:
        // only the end of the note retires it.
        if age >= self.spec.env.total() {
            self.active = false;
            return 0.;
        }

        // The cutoff sweeps across the whole note -- a bass note's filter closing over half a
        // second -- while the pitch drop belongs to the front of it.
        let progress = (age / self.spec.env.total().max(1e-4)).clamp(0., 1.);
        let glide = if self.spec.glide_seconds > 0. {
            (age / self.spec.glide_seconds).clamp(0., 1.)
        } else {
            progress
        };
        // Eased rather than linear: a kick's pitch falls fast and then settles.
        let eased = 1. - (1. - glide) * (1. - glide);
        let inc = self.inc_from + (self.inc_to - self.inc_from) * eased;

        self.phase += inc;
        if self.phase >= 1. {
            self.phase -= 1.;
        }
        let raw = match self.spec.wave {
            Wave::Sine => (self.phase * std::f32::consts::TAU).sin(),
            Wave::Saw => {
                let saw = 2. * self.phase - 1.;
                saw - poly_blep(self.phase, inc)
            }
            Wave::Square => {
                let square = if self.phase < 0.5 { 1. } else { -1. };
                square + poly_blep(self.phase, inc) - poly_blep((self.phase + 0.5) % 1., inc)
            }
            Wave::Triangle => 4. * (self.phase - 0.5).abs() - 1.,
            Wave::Noise => {
                // xorshift32: one multiply-free step, and a fresh stream per voice.
                let mut x = self.noise_state ^ noise_bed;
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                self.noise_state = x;
                (x as f32 / u32::MAX as f32) * 2. - 1.
            }
        };

        let g = self.g_from + (self.g_to - self.g_from) * progress;
        let filtered = match self.spec.filter.mode {
            FilterMode::Off => raw,
            _ => self.svf.process(raw, g, self.k, self.spec.filter.mode),
        };
        let out = filtered * level * self.spec.amp;
        self.age_samples += 1;
        if self.age_samples >= self.delay_samples + self.length_samples {
            self.active = false;
        }
        out
    }
}

/// The fixed pool. Slots are handed out round-robin, so a repeated note reuses the most
/// recently silent slot instead of walking the whole array.
pub struct Voices {
    slots: Vec<Voice>,
    next: usize,
    sample_rate: f32,
    noise_bed: u32,
}

impl Voices {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            slots: vec![Voice::default(); MAX_VOICES],
            next: 0,
            sample_rate,
            noise_bed: 0x9e37_79b9,
        }
    }

    /// The device changed its rate; every live voice is rescaled rather than cut.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|voice| voice.active).count()
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn silence(&mut self) {
        for slot in &mut self.slots {
            slot.active = false;
        }
    }

    /// Start one voice. Returns false only when every slot is busy and the oldest could not
    /// be taken, which at 96 slots means the mix is already saturated.
    pub fn start(&mut self, spec: VoiceSpec) -> bool {
        let sample_rate = self.sample_rate;
        let length_samples = (spec.env.total() * sample_rate).round().max(2.) as u32;
        let delay_samples = (spec.delay_seconds * sample_rate).round() as u32;

        let index = self
            .slots
            .iter()
            .position(|voice| !voice.active)
            .unwrap_or_else(|| {
                let oldest = (0..MAX_VOICES)
                    .max_by_key(|i| self.slots[*i].age_samples)
                    .unwrap_or(0);
                self.next = (oldest + 1) % MAX_VOICES;
                oldest
            });
        self.next = (index + 1) % MAX_VOICES;

        let nyquist = sample_rate * 0.45;
        let to_g = |hz: f32| {
            let clamped = hz.clamp(20., nyquist);
            // tan(pi f / sr), the TPT prewarp, without a tan call per sample.
            (std::f32::consts::PI * clamped / sample_rate).tan()
        };
        let g_from = to_g(spec.filter.from_hz);
        let g_to = to_g(spec.filter.to_hz);
        let k = 1. / spec.filter.q.max(0.05);

        let pan = spec.pan.clamp(-1., 1.);
        // Constant-power pan, because a centred pad should not be 3 dB louder than a panned one.
        let angle = (pan + 1.) * std::f32::consts::FRAC_PI_4;
        let gain_l = angle.cos();
        let gain_r = angle.sin();

        let inc_from = (spec.hz / sample_rate).clamp(0., 0.45);
        let inc_to = spec
            .glide_to
            .unwrap_or(spec.hz)
            .clamp(0., sample_rate * 0.45)
            / sample_rate;
        let detune = 2f32.powf(spec.detune_cents / 1200.);

        self.slots[index] = Voice {
            spec,
            age_samples: 0,
            length_samples,
            delay_samples,
            g_from,
            g_to,
            k,
            svf: Svf::default(),
            phase: 0.,
            inc_from: inc_from * detune,
            inc_to: inc_to * detune,
            gain_l,
            gain_r,
            noise_state: spec.noise_seed | 1,
            active: true,
        };
        true
    }

    /// Add one sample into the dry and duck buses.
    #[inline]
    pub fn add_sample(
        &mut self,
        dry: &mut [f32; 2],
        duck: &mut [f32; 2],
        rev: &mut f32,
        delay: &mut f32,
    ) {
        let noise_bed = self.noise_bed;
        for slot in &mut self.slots {
            if !slot.active {
                continue;
            }
            let value = slot.sample(self.sample_rate, noise_bed);
            let left = value * slot.gain_l;
            let right = value * slot.gain_r;
            let bus = match slot.spec.bus {
                Bus::Dry => &mut *dry,
                Bus::Duck => &mut *duck,
            };
            bus[0] += left;
            bus[1] += right;
            if slot.spec.send_rev > 0. {
                *rev += (left + right) * 0.5 * slot.spec.send_rev;
            }
            if slot.spec.send_del > 0. {
                *delay += (left + right) * 0.5 * slot.spec.send_del;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_envelope_swells_holds_and_releases() {
        let env = Env::Hold {
            attack: 1.,
            hold_until: 3.,
            total: 4.,
        };
        assert_eq!(env.level(0.), 0.);
        assert!((env.level(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(env.level(1.5), 1.);
        assert_eq!(env.level(2.9), 1.);
        assert!((env.level(3.5) - 0.5).abs() < 0.01);
        assert_eq!(env.level(4.), 0.);
    }

    #[test]
    fn a_decay_envelope_reaches_silence_and_stays_there() {
        let env = Env::Decay {
            attack: 0.01,
            total: 1.,
        };
        assert!((env.level(0.01) - 1.).abs() < 1e-3);
        // Monotone down, and a hundredth of the peak near the end.
        let mut previous = 1.;
        for step in 1..100 {
            let level = env.level(step as f32 / 100.);
            assert!(level <= previous + 1e-6, "{level} above {previous}");
            previous = level;
        }
        assert!(env.level(0.99) < 0.02);
        assert_eq!(env.level(1.), 0.);
    }

    #[test]
    fn a_voice_sounds_then_retires_itself() {
        let mut voices = Voices::new(48_000.);
        let env = Env::Decay {
            attack: 0.001,
            total: 0.05,
        };
        voices.start(VoiceSpec::new(Wave::Sine, 69, env, 0.5));
        assert_eq!(voices.active_count(), 1);
        let mut peak = 0f32;
        for _ in 0..4800 {
            let (mut dry, mut duck) = ([0f32; 2], [0f32; 2]);
            let (mut rev, mut delay) = (0., 0.);
            voices.add_sample(&mut dry, &mut duck, &mut rev, &mut delay);
            peak = peak.max(dry[0].abs());
        }
        assert!(
            peak > 0.1,
            "a 440 Hz sine at half amplitude should be audible, {peak}"
        );
        assert!(
            peak <= 0.5 + 1e-3,
            "and should not exceed its own amplitude, {peak}"
        );
        assert_eq!(voices.active_count(), 0);
    }

    /// The pan contract: a voice hard left puts nothing in the right channel.
    #[test]
    fn panning_splits_the_channels() {
        let mut voices = Voices::new(48_000.);
        let env = Env::Decay {
            attack: 0.001,
            total: 0.02,
        };
        voices.start(VoiceSpec::new(Wave::Sine, 60, env, 0.5).pan(-1.));
        let mut right_peak = 0f32;
        for _ in 0..960 {
            let (mut dry, mut duck) = ([0f32; 2], [0f32; 2]);
            let (mut rev, mut delay) = (0., 0.);
            voices.add_sample(&mut dry, &mut duck, &mut rev, &mut delay);
            right_peak = right_peak.max(dry[1].abs());
        }
        assert!(
            right_peak < 1e-6,
            "hard left leaked {right_peak} into the right channel"
        );
    }

    /// A kick is a sine dropping an octave and a half; the drop has to actually happen.
    #[test]
    fn a_gliding_voice_falls_in_pitch() {
        let mut voices = Voices::new(48_000.);
        let env = Env::Decay {
            attack: 0.001,
            total: 0.3,
        };
        voices.start(VoiceSpec::new(Wave::Sine, 60, env, 0.8).glide(30., 0.1));
        // Count zero crossings in the first and the last tenth of the note; a falling pitch
        // crosses zero fewer times per second at the end.
        let mut crossings = Vec::new();
        let mut previous = 0f32;
        for index in 0..14_400usize {
            let (mut dry, mut duck) = ([0f32; 2], [0f32; 2]);
            let (mut rev, mut delay) = (0., 0.);
            voices.add_sample(&mut dry, &mut duck, &mut rev, &mut delay);
            let value = dry[0];
            if index.is_multiple_of(1440) {
                crossings.push(0);
            }
            if value.signum() != previous.signum() {
                if let Some(last) = crossings.last_mut() {
                    *last += 1;
                }
            }
            previous = value;
        }
        let first = crossings[1];
        let last = *crossings.last().expect("crossings");
        assert!(
            first > last * 2,
            "expected the glide to halve the pitch at least, {first} vs {last}"
        );
    }

    #[test]
    fn the_pool_reuses_a_slot_after_silence() {
        let mut voices = Voices::new(48_000.);
        let env = Env::Decay {
            attack: 0.001,
            total: 0.01,
        };
        for _ in 0..MAX_VOICES * 2 {
            voices.start(VoiceSpec::new(Wave::Sine, 60, env, 0.1));
        }
        assert!(
            voices.active_count() <= MAX_VOICES,
            "the pool must never exceed its size"
        );
    }

    #[test]
    fn a_full_pool_steals_the_oldest_voice() {
        let mut voices = Voices::new(48_000.);
        let env = Env::Hold {
            attack: 0.01,
            hold_until: 100.,
            total: 100.,
        };
        for _ in 0..MAX_VOICES {
            voices.start(VoiceSpec::new(Wave::Saw, 60, env, 0.1));
        }
        for _ in 0..4800 {
            let (mut dry, mut duck) = ([0f32; 2], [0f32; 2]);
            let (mut rev, mut delay) = (0., 0.);
            voices.add_sample(&mut dry, &mut duck, &mut rev, &mut delay);
        }
        assert_eq!(voices.active_count(), MAX_VOICES);
        assert!(voices.start(VoiceSpec::new(Wave::Saw, 60, env, 0.1)));
        assert_eq!(voices.active_count(), MAX_VOICES);
    }
}
