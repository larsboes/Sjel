//! What the visuals read: a spectrum, a level, and nothing else.
//!
//! The window draws the *sound*, not a simulation of it, so the numbers have to leave the audio
//! thread without blocking it. A radix-2 FFT over a decimated 1024-sample window, run about
//! sixty times a second, is cheap enough to sit inside the audio callback: roughly twenty
//! thousand operations per update, which at 96 kHz is under half a percent of one core.
//!
//! The decimation is deliberate and its cost is known: a one-pole lowpass at 8 kHz first, then
//! every fourth sample, so the bands above 8 kHz are progressively quieter than they are in the
//! mix. A spectrum that is honest to within a few decibels at the top is the right trade for
//! never stalling a buffer.

use super::control::BANDS;

/// Points in the transform. 1024 at a quarter of the device rate is about 23 Hz per bin, which
/// is fine enough to keep the low bands from merging.
const POINTS: usize = 1024;
/// A Hann window, so a steady tone does not smear across every band.
fn hann(index: usize) -> f32 {
    let phase = std::f32::consts::TAU * index as f32 / POINTS as f32;
    0.5 - 0.5 * phase.cos()
}

/// The band edges, in hertz: logarithmic from `LOWEST` to `HIGHEST`, which is how an octave is
/// perceived rather than how a linear axis is drawn.
const LOWEST: f32 = 55.;
const HIGHEST: f32 = 12_000.;

pub struct Analyser {
    /// Raw incoming samples, one per device frame.
    decimation: usize,
    decimation_count: usize,
    /// One-pole lowpass state, anti-aliasing for the decimator.
    lowpass: f32,
    lowpass_coefficient: f32,
    /// Decimated samples, the transform's input.
    window: [f32; POINTS],
    filled: usize,
    /// Scratch for the transform, so nothing is allocated per update.
    real: [f32; POINTS],
    imaginary: [f32; POINTS],
    twiddle_cos: [f32; POINTS / 2],
    twiddle_sin: [f32; POINTS / 2],
    window_table: [f32; POINTS],
    /// Smoothed band magnitudes, in `0.0..=1.0`.
    bands: [f32; BANDS],
    /// How much the bands rise instantly and fall slowly, which is what a meter feels like.
    release: f32,
    sample_rate: f32,
    /// Running level.
    sum_squares: f64,
    count: u32,
    rms: f32,
    peak: f32,
}

impl Analyser {
    pub fn new(sample_rate: f32) -> Self {
        debug_assert!(sample_rate > 8_000., "an analyser needs a usable rate");
        let decimation = ((sample_rate / 24_000.).round() as usize).max(1);
        let effective = sample_rate / decimation as f32;
        // 8 kHz corner, or just under Nyquist when the device is slow.
        let corner = 8_000f32.min(effective * 0.45);
        let lowpass_coefficient = 1. - (-std::f32::consts::TAU * corner / sample_rate).exp();
        Self {
            decimation,
            decimation_count: 0,
            lowpass: 0.,
            lowpass_coefficient,
            window: [0.; POINTS],
            filled: 0,
            real: [0.; POINTS],
            imaginary: [0.; POINTS],
            twiddle_cos: std::array::from_fn(|index| {
                (std::f32::consts::TAU * index as f32 / POINTS as f32).cos()
            }),
            twiddle_sin: std::array::from_fn(|index| {
                -(std::f32::consts::TAU * index as f32 / POINTS as f32).sin()
            }),
            window_table: std::array::from_fn(hann),
            bands: [0.; BANDS],
            // A tenth of a second to fall to a third of a peak: fast enough to follow a hat,
            // slow enough that a band does not flicker.
            release: (-1. / 6.0f32).exp(),
            sample_rate,
            sum_squares: 0.,
            count: 0,
            rms: 0.,
            peak: 0.,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        *self = Self::new(sample_rate);
    }

    pub fn bands(&self) -> [f32; BANDS] {
        self.bands
    }

    pub fn rms(&self) -> f32 {
        self.rms
    }

    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// One frame of one channel.
    pub fn push(&mut self, sample: f32) {
        self.sum_squares += (sample as f64) * (sample as f64);
        self.count += 1;
        self.peak = self.peak.max(sample.abs());

        self.lowpass += self.lowpass_coefficient * (sample - self.lowpass);
        self.decimation_count += 1;
        if self.decimation_count < self.decimation {
            return;
        }
        self.decimation_count = 0;

        self.window[self.filled] = self.lowpass;
        self.filled += 1;
        if self.filled == POINTS {
            self.filled = 0;
            self.analyse();
        }
    }

    /// Take whatever the analyser has measured. Called by the audio thread about sixty times a
    /// second; the level is reset here so a meter reads "since the last look".
    pub fn take_level(&mut self) -> (f32, f32) {
        self.rms = if self.count > 0 {
            (self.sum_squares / self.count as f64).sqrt() as f32
        } else {
            0.
        };
        self.sum_squares = 0.;
        self.count = 0;
        let peak = self.peak;
        self.peak *= 0.5;
        (self.rms, peak)
    }

    /// Whether the caller should look at the bands again.
    pub fn frames_per_update(sample_rate: f32) -> usize {
        (sample_rate / 60.).max(1.) as usize
    }

    fn analyse(&mut self) {
        // Window in, and clear the imaginary part.
        for index in 0..POINTS {
            let sample = self.window[(self.filled + index) % POINTS];
            self.real[index] = sample * self.window_table[index];
            self.imaginary[index] = 0.;
        }
        self.transform();

        // Fold bins into logarithmic bands, keeping the loudest bin in each.
        let bin_hz = self.sample_rate / self.decimation as f32 / POINTS as f32;
        let ratio = (HIGHEST / LOWEST).powf(1. / BANDS as f32);
        let mut fresh = [0f32; BANDS];
        for (band, slot) in fresh.iter_mut().enumerate() {
            let low_hz = LOWEST * ratio.powi(band as i32);
            let high_hz = low_hz * ratio;
            let low_bin = ((low_hz / bin_hz).floor() as usize).clamp(1, POINTS / 2 - 1);
            let high_bin = ((high_hz / bin_hz).ceil() as usize).clamp(low_bin + 1, POINTS / 2);
            let mut magnitude = 0f32;
            for bin in low_bin..high_bin {
                let re = self.real[bin];
                let im = self.imaginary[bin];
                magnitude = magnitude.max((re * re + im * im).sqrt());
            }
            // Two corrections, both for the eye rather than the ear.
            //
            // A mix carries most of its energy below a few hundred hertz, so a flat spectrum
            // draws only the first three ridges and leaves the rest dead. The tilt lifts the top
            // by 1.5 dB per octave, which is the same thing every music visualiser does and is
            // why they look like the music sounds.
            //
            // The reference then puts a healthy mix in the middle of the bar. A single bin of a
            // full-scale tone reads about 256 here; a dense mix spreads that energy over hundreds
            // of bins, so the anchor is the mix and not the tone. The square root makes the low
            // end of that range visible, which a linear axis does not.
            let tilt = (low_hz / LOWEST).powf(0.25).min(4.);
            let scaled = ((magnitude * tilt) / 6.).sqrt().clamp(0., 1.);
            // Instant rise, slow fall.
            *slot = if scaled > self.bands[band] {
                scaled
            } else {
                scaled + (self.bands[band] - scaled) * self.release
            };
        }
        self.bands = fresh;
    }

    /// In-place iterative radix-2 Cooley-Tukey. Bit-reversal first, then butterflies.
    fn transform(&mut self) {
        let mut j = 0usize;
        for i in 1..POINTS {
            let mut bit = POINTS >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                self.real.swap(i, j);
                self.imaginary.swap(i, j);
            }
        }
        let mut length = 2usize;
        while length <= POINTS {
            let half = length / 2;
            let stride = POINTS / length;
            let mut start = 0usize;
            while start < POINTS {
                for offset in 0..half {
                    let twiddle = offset * stride;
                    let (cos, sin) = (self.twiddle_cos[twiddle], self.twiddle_sin[twiddle]);
                    let a = start + offset;
                    let b = a + half;
                    let re = self.real[b] * cos - self.imaginary[b] * sin;
                    let im = self.real[b] * sin + self.imaginary[b] * cos;
                    self.real[b] = self.real[a] - re;
                    self.imaginary[b] = self.imaginary[a] - im;
                    self.real[a] += re;
                    self.imaginary[a] += im;
                }
                start += length;
            }
            length <<= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push `seconds` of a tone and return the settled bands.
    fn bands_for(
        analyser: &mut Analyser,
        hertz: f32,
        sample_rate: f32,
        seconds: f32,
    ) -> [f32; BANDS] {
        let frames = (sample_rate * seconds) as usize;
        for index in 0..frames {
            let phase = std::f32::consts::TAU * hertz * index as f32 / sample_rate;
            analyser.push(phase.sin() * 0.5);
        }
        analyser.bands()
    }

    fn loudest_band(bands: &[f32; BANDS]) -> usize {
        let mut best = 0;
        for index in 1..BANDS {
            if bands[index] > bands[best] {
                best = index;
            }
        }
        best
    }

    /// The band that lights up has to be the band the tone is in: 440 Hz sits low, 4 kHz high.
    #[test]
    fn a_tone_lands_in_its_own_band() {
        let mut low = Analyser::new(48_000.);
        let bands = bands_for(&mut low, 220., 48_000., 1.);
        let low_band = loudest_band(&bands);

        let mut high = Analyser::new(48_000.);
        let bands = bands_for(&mut high, 4_000., 48_000., 1.);
        let high_band = loudest_band(&bands);

        assert!(
            high_band > low_band + 4,
            "220 Hz landed in {low_band}, 4 kHz in {high_band}"
        );
    }

    /// Silence reads as silence rather than as a leftover of the last tone.
    #[test]
    fn silence_decays_to_nothing() {
        let mut analyser = Analyser::new(48_000.);
        bands_for(&mut analyser, 440., 48_000., 1.);
        let loud = analyser.bands();
        for _ in 0..(48_000 * 3) {
            analyser.push(0.);
        }
        let quiet = analyser.bands();
        assert!(
            loud.iter().sum::<f32>() > quiet.iter().sum::<f32>() * 20.,
            "a tone then silence should not look the same"
        );
        assert!(quiet.iter().all(|band| *band < 0.05), "{quiet:?}");
    }

    /// The level reading is per-look, not cumulative.
    #[test]
    fn the_level_resets_between_reads() {
        let mut analyser = Analyser::new(48_000.);
        for _ in 0..4_800 {
            analyser.push(0.5);
        }
        let (rms, peak) = analyser.take_level();
        assert!((rms - 0.5).abs() < 1e-3, "{rms}");
        assert!((peak - 0.5).abs() < 1e-3, "{peak}");
        let (rms, _) = analyser.take_level();
        assert_eq!(rms, 0.);
    }

    /// A device at 96 kHz is the normal case on this machine; the analyser has to work there.
    #[test]
    fn a_high_sample_rate_still_resolves_bands() {
        let mut analyser = Analyser::new(96_000.);
        let bands = bands_for(&mut analyser, 1_000., 96_000., 1.);
        let band = loudest_band(&bands);
        // The axis runs 55 Hz to 12 kHz in 32 log bands, so 1 kHz belongs at about band 17.
        assert!((14..21).contains(&band), "1 kHz landed in band {band}");
        assert!(bands[band] > 0.2, "{:?}", bands[band]);
    }
}
