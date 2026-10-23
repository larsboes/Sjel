//! The four buses and the master chain: room, echo, sidechain, compressor, limiter.
//!
//! The browser gives each scene two convolvers holding generated noise impulses (1.2 s to 6 s),
//! two of them so the room can change size without a click. This is the one place the port
//! deliberately does not follow: a six-second convolution is a 12-second FFT reverb, and the
//! perceptual job here -- "a bigger `space` sounds like a bigger room, and it keeps ringing
//! after the note" -- is what Freeverb's comb-and-allpass network does at a fraction of the
//! cost. The substitution is real and is written down here rather than discovered later:
//!
//! - `space` 0..1 drives comb feedback, so the tail grows from about 1 s to about 6 s.
//! - `brightness` drives the damping, so a dark preset has a dark room.
//! - The crossfade between two rooms becomes a smooth parameter change, which is strictly
//!   fewer artefacts than two convolvers and a gain ramp.
//!
//! Everything else is the browser's: the echo is a stereo feedback delay at three sixteenths
//! of a beat, the sidechain ducks to 0.22 and recovers over 300 ms on every kick, and the
//! master is volume squared into a -14 dB / 4:1 compressor.

/// The room. One delay line per comb and allpass, per channel.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
/// Tuning above is for 44.1 kHz; other rates scale it.
const TUNING_RATE: f32 = 44_100.;
const STEREO_SPREAD: usize = 23;

#[derive(Clone, Debug)]
struct Comb {
    buffer: Vec<f32>,
    index: usize,
    length: usize,
    feedback: f32,
    damp: f32,
    store: f32,
}

impl Comb {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            index: 0,
            length: 1,
            feedback: 0.8,
            damp: 0.3,
            store: 0.,
        }
    }

    /// Sized for the device's own rate. A fixed 44.1 kHz buffer would clamp every comb to the
    /// same length on a 96 kHz device, which turns the room into one comb filter.
    fn set(&mut self, tuning: usize, sample_rate: f32, feedback: f32, damp: f32) {
        let scaled = ((tuning as f32 * sample_rate / TUNING_RATE) as usize).max(1);
        if self.buffer.len() < scaled {
            self.buffer.resize(scaled, 0.);
        }
        self.length = scaled;
        self.index %= self.length;
        self.feedback = feedback;
        self.damp = damp;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.store = output * (1. - self.damp) + self.store * self.damp;
        self.buffer[self.index] = input + self.store * self.feedback;
        self.index += 1;
        if self.index >= self.length {
            self.index = 0;
        }
        output
    }

    fn clear(&mut self) {
        self.buffer.iter_mut().for_each(|sample| *sample = 0.);
        self.index = 0;
        self.store = 0.;
    }
}

#[derive(Clone, Debug)]
struct Allpass {
    buffer: Vec<f32>,
    index: usize,
    length: usize,
}

impl Allpass {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            index: 0,
            length: 1,
        }
    }

    fn set(&mut self, tuning: usize, sample_rate: f32) {
        let scaled = ((tuning as f32 * sample_rate / TUNING_RATE) as usize).max(1);
        if self.buffer.len() < scaled {
            self.buffer.resize(scaled, 0.);
        }
        self.length = scaled;
        self.index %= self.length;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = buffered - input;
        self.buffer[self.index] = input + buffered * 0.5;
        self.index += 1;
        if self.index >= self.length {
            self.index = 0;
        }
        output
    }

    fn clear(&mut self) {
        self.buffer.iter_mut().for_each(|sample| *sample = 0.);
        self.index = 0;
    }
}

/// Freeverb, in stereo, with `space` on the tail length and `brightness` on the damping.
pub struct Room {
    combs_l: [Comb; 8],
    combs_r: [Comb; 8],
    allpass_l: [Allpass; 4],
    allpass_r: [Allpass; 4],
    sample_rate: f32,
    space: f32,
    damping: f32,
}

impl Room {
    pub fn new(sample_rate: f32) -> Self {
        let mut room = Self {
            combs_l: std::array::from_fn(|_| Comb::new()),
            combs_r: std::array::from_fn(|_| Comb::new()),
            allpass_l: std::array::from_fn(|_| Allpass::new()),
            allpass_r: std::array::from_fn(|_| Allpass::new()),
            sample_rate,
            space: -1.,
            damping: -1.,
        };
        room.set_sample_rate(sample_rate);
        room.set_space(0.5, 0.5);
        room
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        for (index, tuning) in ALLPASS_TUNING.iter().enumerate() {
            self.allpass_l[index].set(*tuning, sample_rate);
            self.allpass_r[index].set(tuning + STEREO_SPREAD, sample_rate);
        }
        // Feedback is a property of `space`, so re-derive it at the new rate rather than
        // resetting to a default.
        let (space, damping) = (self.space.max(0.), self.damping.max(0.));
        self.set_space(space, damping);
    }

    /// `space` 0..1 grows the tail; `brightness` 0..1 opens the damping.
    pub fn set_space(&mut self, space: f32, brightness: f32) {
        // 0.78 at nothing, 0.94 at everything: roughly 1 s to 6 s of tail.
        let feedback = 0.78 + 0.16 * space.clamp(0., 1.);
        let damp = (0.55 - 0.45 * brightness.clamp(0., 1.)).clamp(0.05, 0.9);
        for (index, tuning) in COMB_TUNING.iter().enumerate() {
            self.combs_l[index].set(*tuning, self.sample_rate, feedback, damp);
            self.combs_r[index].set(tuning + STEREO_SPREAD, self.sample_rate, feedback, damp);
        }
        self.space = space;
        self.damping = brightness;
    }

    /// The prewarp above, for callers that want to know whether a change is worth applying.
    pub fn space(&self) -> f32 {
        self.space
    }

    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        let feed = (input[0] + input[1]) * 0.015;
        let mut out = [0f32; 2];
        for index in 0..8 {
            out[0] += self.combs_l[index].process(feed);
            out[1] += self.combs_r[index].process(feed);
        }
        for index in 0..4 {
            out[0] = self.allpass_l[index].process(out[0]);
            out[1] = self.allpass_r[index].process(out[1]);
        }
        out
    }

    pub fn clear(&mut self) {
        for comb in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            comb.clear();
        }
        for allpass in self.allpass_l.iter_mut().chain(self.allpass_r.iter_mut()) {
            allpass.clear();
        }
    }
}

/// The echo: a stereo feedback delay whose time follows the tempo.
///
/// The delay time moves through a one-pole rather than jumping, and the read is interpolated. A
/// scape's tempo follows the `pace` parameter, which drifts on its own LFO, so the delay time is
/// retuned several times a minute; setting it outright stepped the read position and clicked every
/// time it moved.
pub struct Echo {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write: usize,
    /// Where the read is now, in samples, and where it is heading.
    delay: f32,
    target: f32,
    /// One-pole coefficient for the move, about a fifth of a second.
    smoothing: f32,
    feedback: f32,
    wet: f32,
    sample_rate: f32,
}

impl Echo {
    pub fn new(sample_rate: f32) -> Self {
        let mut echo = Self {
            buffer_l: vec![0.; (sample_rate * 2.) as usize],
            buffer_r: vec![0.; (sample_rate * 2.) as usize],
            write: 0,
            delay: sample_rate * 0.35,
            target: sample_rate * 0.35,
            smoothing: 0.,
            feedback: 0.32,
            wet: 0.5,
            sample_rate,
        };
        echo.set_sample_rate(sample_rate);
        echo
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.buffer_l = vec![0.; (sample_rate * 2.) as usize];
        self.buffer_r = vec![0.; (sample_rate * 2.) as usize];
        self.write = 0;
        self.smoothing = 1. - (-1. / (0.2 * sample_rate)).exp();
        let seconds = self.target / self.sample_rate.max(1.);
        self.snap_delay_seconds(seconds);
    }

    /// The browser puts the echo at three sixteenths of a beat.
    ///
    /// The move is a glide, for a tempo that drifts. A *new* preset has to arrive at its own delay
    /// immediately, or the echo spends half a second bending into place -- that is what
    /// [`Self::snap_delay_seconds`] is for.
    pub fn set_delay_seconds(&mut self, seconds: f32) {
        self.target = self.seconds_to_samples(seconds);
    }

    /// Move the delay at once, for a change that is not a drift.
    pub fn snap_delay_seconds(&mut self, seconds: f32) {
        self.target = self.seconds_to_samples(seconds);
        self.delay = self.target;
    }

    fn seconds_to_samples(&self, seconds: f32) -> f32 {
        let max = (self.buffer_l.len().saturating_sub(2)).max(1) as f32;
        (seconds * self.sample_rate).clamp(1., max)
    }

    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        let length = self.buffer_l.len();
        self.delay += self.smoothing * (self.target - self.delay);
        // Interpolated read, so a moving delay bends the tail instead of stepping through it.
        let mut position = self.write as f32 - self.delay;
        while position < 0. {
            position += length as f32;
        }
        let first = position as usize % length;
        let second = (first + 1) % length;
        let fraction = position - position.floor();
        let out_l = self.buffer_l[first] * (1. - fraction) + self.buffer_l[second] * fraction;
        let out_r = self.buffer_r[first] * (1. - fraction) + self.buffer_r[second] * fraction;
        // The right channel feeds back into the left, so a repeat crosses the field.
        self.buffer_l[self.write] = input[0] + out_r * self.feedback;
        self.buffer_r[self.write] = input[1] + out_l * self.feedback;
        self.write = (self.write + 1) % length;
        [out_l * self.wet, out_r * self.wet]
    }

    pub fn clear(&mut self) {
        self.buffer_l.iter_mut().for_each(|sample| *sample = 0.);
        self.buffer_r.iter_mut().for_each(|sample| *sample = 0.);
    }
}

/// The sidechain: 0.22 on the kick, linear back to unity over 300 ms.
pub struct Duck {
    level: f32,
    step: f32,
    recovering: bool,
}

impl Duck {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            level: 1.,
            step: 1. / (0.3 * sample_rate),
            recovering: false,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.step = 1. / (0.3 * sample_rate);
    }

    pub fn trigger(&mut self) {
        self.level = 0.22;
        self.recovering = true;
    }

    #[inline]
    pub fn apply(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.recovering {
            self.level += self.step;
            if self.level >= 1. {
                self.level = 1.;
                self.recovering = false;
            }
        }
        [input[0] * self.level, input[1] * self.level]
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}

/// Volume squared, a -14 dB / 4:1 peak compressor and a soft limiter, in that order, which is
/// the browser's master chain -- plus two things a native build needs and a browser gets for free.
///
/// The gain is smoothed per sample, because a dragged volume slider arrives as a new number every
/// block and a gain that steps at block boundaries is a zipper. And the whole mix is trimmed: the
/// browser's own voice levels leave about ten decibels of headroom unused, so the compressor sat on
/// top of the entire mix instead of catching transients. `MIX_TRIM` spends that headroom.
pub struct Master {
    volume: f32,
    gain: f32,
    gain_smoothing: f32,
    envelope: f32,
    attack: f32,
    release: f32,
    sample_rate: f32,
    peak: f32,
}

/// Where the mix sits before the compressor. Chosen so the loudest preset at full volume peaks
/// near -6 dBFS: the compressor then moves on transients and the limiter never works at all.
const MIX_TRIM: f32 = 1.8;

impl Master {
    pub fn new(sample_rate: f32) -> Self {
        let mut master = Self {
            volume: 0.65,
            gain: 0.65 * 0.65 * MIX_TRIM,
            gain_smoothing: 0.,
            envelope: 0.,
            attack: 0.,
            release: 0.,
            sample_rate,
            peak: 1.,
        };
        master.set_sample_rate(sample_rate);
        master.set_volume(0.65);
        master
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        // 5 ms of attack, 120 ms of release: fast enough to catch a kick, slow enough that it
        // does not pump on its own.
        self.attack = (-1. / (0.005 * sample_rate)).exp();
        self.release = (-1. / (0.12 * sample_rate)).exp();
        // 20 ms to reach a new volume: long enough to be inaudible, short enough to feel immediate.
        self.gain_smoothing = 1. - (-1. / (0.02 * sample_rate)).exp();
    }

    /// The browser squares the volume, so a mid slider is quieter than a linear one.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0., 1.);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Output level of the last sample, for the meter. Already post-limiter.
    pub fn peak(&self) -> f32 {
        self.peak
    }

    #[inline]
    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        let target = self.volume * self.volume * MIX_TRIM;
        self.gain += self.gain_smoothing * (target - self.gain);
        let mut out = [input[0] * self.gain, input[1] * self.gain];

        // Peak detector with a 5 ms attack and a 120 ms release.
        let peak = out[0].abs().max(out[1].abs());
        let coefficient = if peak > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope = peak + (self.envelope - peak) * coefficient;
        self.peak = self.envelope;

        // -14 dB threshold, 4:1 above it.
        const THRESHOLD: f32 = 0.1995; // 10^(-14/20)
        if self.envelope > THRESHOLD {
            let over = self.envelope / THRESHOLD;
            let compressed = THRESHOLD * over.powf(1. / 4.);
            let reduction = compressed / self.envelope;
            out[0] *= reduction;
            out[1] *= reduction;
        }

        // Soft limiter: linear to 0.8, then a curve that cannot pass 1.0.
        [limit(out[0]), limit(out[1])]
    }
}

#[inline]
fn limit(value: f32) -> f32 {
    const KNEE: f32 = 0.8;
    if value.abs() <= KNEE {
        return value;
    }
    let sign = if value < 0. { -1. } else { 1. };
    let over = value.abs() - KNEE;
    sign * (KNEE + (1. - KNEE) * (over / (over + (1. - KNEE))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limiter_never_passes_unity() {
        for step in 0..2000 {
            let value = step as f32 / 100.;
            assert!(limit(value).abs() < 1.0, "{value}");
            assert!(limit(-value).abs() < 1.0, "{value}");
        }
        // And it is transparent on the way up to the knee.
        assert!((limit(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_master_is_silent_at_zero_volume() {
        let mut master = Master::new(48_000.);
        master.set_volume(0.);
        for _ in 0..4_800 {
            master.process([0.9, -0.9]);
        }
        for _ in 0..24_000 {
            master.process([0.9, -0.9]);
        }
        let out = master.process([0.9, -0.9]);
        assert!(out[0].abs() < 1e-3, "{out:?}");
    }

    /// A volume change arrives every block while a slider is dragged; the gain has to walk to it
    /// rather than step, or the drag is a zipper.
    #[test]
    fn a_volume_change_is_smoothed_rather_than_stepped() {
        // A quiet signal, so what the test measures is the gain and not the compressor on top of it.
        let mut master = Master::new(48_000.);
        master.set_volume(0.);
        for _ in 0..4_800 {
            master.process([0.001, 0.001]);
        }
        master.set_volume(1.);
        let first = master.process([0.001, 0.001])[0];
        assert!(first < 0.000_1, "the gain stepped to {first} in one sample");
        let mut previous = first;
        let mut biggest = 0f32;
        for _ in 0..4_800 {
            let value = master.process([0.001, 0.001])[0];
            biggest = biggest.max((value - previous).abs());
            previous = value;
        }
        assert!(
            biggest < 0.000_01,
            "the gain moved by {biggest} between samples"
        );
        assert!(previous > 0.001_5, "the gain never arrived: {previous}");
    }

    #[test]
    fn the_sidechain_ducks_then_recovers() {
        let mut duck = Duck::new(48_000.);
        assert_eq!(duck.apply([1., 1.]), [1., 1.]);
        duck.trigger();
        let ducked = duck.apply([1., 1.]);
        assert!(ducked[0] < 0.3, "{ducked:?}");
        for _ in 0..19_200 {
            duck.apply([1., 1.]);
        }
        assert!((duck.apply([1., 1.])[0] - 1.).abs() < 1e-6);
    }

    /// The room has to ring: an impulse in produces signal long after it.
    #[test]
    fn the_room_rings_and_then_decays() {
        let mut room = Room::new(48_000.);
        room.set_space(0.8, 0.5);
        room.process([1., 1.]);
        let mut late = 0f32;
        for step in 0..(48_000 * 2) as usize {
            let out = room.process([0., 0.]);
            if step > 9_600 {
                late = late.max(out[0].abs());
            }
        }
        assert!(late > 1e-5, "the tail died immediately: {late}");
        // A longer space keeps more of it than a short one.
        let mut short = Room::new(48_000.);
        short.set_space(0., 0.5);
        short.process([1., 1.]);
        let mut short_late = 0f32;
        for step in 0..(48_000 * 2) as usize {
            let out = short.process([0., 0.]);
            if step > 9_600 {
                short_late = short_late.max(out[0].abs());
            }
        }
        assert!(late > short_late, "{late} vs {short_late}");
    }

    #[test]
    fn the_echo_repeats_at_the_delay_time() {
        let mut echo = Echo::new(48_000.);
        echo.snap_delay_seconds(0.1);
        echo.process([1., 0.]);
        let mut first_repeat = None;
        for step in 1..9_600 {
            let out = echo.process([0., 0.]);
            if out[0].abs() > 1e-4 && first_repeat.is_none() {
                first_repeat = Some(step);
            }
        }
        let first_repeat = first_repeat.expect("a repeat");
        assert!(
            (first_repeat as i64 - 4_800).abs() <= 2,
            "expected the repeat at 0.1 s, got {first_repeat} samples"
        );
    }
}
