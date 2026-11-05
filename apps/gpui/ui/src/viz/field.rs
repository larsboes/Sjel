//! Audio-driven field physics, stratum level tracking, and motion advances.

use sjel_gpui_core::soundscape::{Preset, Visual, BANDS};

use super::constants::{COMETS, QUIET, STRATA};
use super::types::{approach, Comet, Ring};

/// The field's own state: what it has already reacted to, and how far each ridge has drifted.
pub struct Field {
    /// Amplitude per ridge, smoothed towards its band's own level.
    pub(crate) levels: [f32; STRATA],
    /// Drift phase per ridge, in turns.
    pub(crate) phases: [f32; STRATA],
    /// The swell a kick leaves behind, which is smoother than moving the ridges themselves.
    pub(crate) swell: f32,
    /// The palette actually drawn, moving towards the preset's own so a preset change is a fade
    /// rather than a jump.
    pub(crate) palette: [[f32; 3]; 4],
    pub(crate) palette_set: bool,
    pub(crate) comets: [Comet; COMETS],
    pub(crate) rings: [Ring; STRATA],
    /// The mix's level, smoothed for the rule at the top.
    pub(crate) level: f32,
    /// The event counters the last frame saw, so a kick is one ring and not one per frame.
    pub(crate) kicks_seen: u32,
    pub(crate) notes_seen: u32,
    pub(crate) next_comet: usize,
}

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

impl Field {
    pub fn new() -> Self {
        Self {
            levels: [0.; STRATA],
            // A different phase per ridge, so they never breathe in unison.
            phases: std::array::from_fn(|index| index as f32 * 0.37),
            swell: 0.,
            palette: [[0.; 3]; 4],
            palette_set: false,
            comets: [Comet::default(); COMETS],
            rings: [Ring::default(); STRATA],
            level: 0.,
            kicks_seen: 0,
            notes_seen: 0,
            next_comet: 0,
        }
    }

    /// Read the synth, advance the motion, and report whether anything is still moving. The window
    /// uses that to stop asking for frames once the sound has settled.
    pub fn advance(&mut self, visual: &Visual, preset: Preset, dt: f32) -> bool {
        let dt = dt.clamp(0., 0.25);
        let bands = visual.bands();

        let ratio = BANDS as f32 / STRATA as f32;
        for (index, level) in self.levels.iter_mut().enumerate() {
            // Each ridge carries the loudest band in its slice: a ridge that averaged its slice
            // would go flat as soon as two bands disagreed.
            let low = (index as f32 * ratio) as usize;
            let high = (((index + 1) as f32 * ratio) as usize).clamp(low + 1, BANDS);
            let target = bands[low..high]
                .iter()
                .fold(0f32, |best, band| best.max(*band));
            // Quick to rise, slow to fall: a ridge that dropped as fast as its band flickered.
            let tau = if target > *level { 0.07 } else { 0.34 };
            *level = approach(*level, target, dt, tau);
            // Slow, and different per ridge, so the stack never marches in step.
            self.phases[index] = (self.phases[index] + dt * (0.026 + 0.009 * index as f32)) % 1.;
        }

        self.level = approach(self.level, visual.rms(), dt, 0.09);
        // The swell a kick leaves: a short push, then a calm fall.
        self.swell = approach(self.swell, 0., dt, 0.22);

        // A preset change is a colour fade, not a cut.
        let target = preset.palette();
        if !self.palette_set {
            self.palette = target;
            self.palette_set = true;
        }
        for (channel, colour) in self.palette.iter_mut().zip(target) {
            for (value, wanted) in channel.iter_mut().zip(colour) {
                *value = approach(*value, wanted, dt, 0.5);
            }
        }

        // A kick: one ring per event, and one swell for the whole field. The earlier version jolted
        // every ridge's drift phase instead, which read as a stutter rather than a beat.
        let kicks = visual.kicks();
        if kicks != self.kicks_seen {
            let fresh = kicks.wrapping_sub(self.kicks_seen).min(4);
            self.kicks_seen = kicks;
            for _ in 0..fresh {
                if let Some(ring) = self.rings.iter_mut().find(|ring| !ring.active) {
                    ring.active = true;
                    ring.age = 0.;
                    ring.strength = 1.;
                }
            }
            self.swell = (self.swell + 0.7 * fresh.min(2) as f32).min(1.);
        }

        // A note: a comet on the ridge its pitch class belongs to, starting at its pan.
        let notes = visual.notes();
        if notes != self.notes_seen {
            self.notes_seen = notes;
            let ridge = (visual.midi().rem_euclid(12) as usize * STRATA) / 12;
            let pan = visual.pan();
            self.comets[self.next_comet] = Comet {
                active: true,
                age: 0.,
                stratum: ridge.min(STRATA - 1),
                from: 0.5 + pan * 0.45,
                span: if pan < 0. { 0.5 } else { -0.5 },
                strength: 0.85,
            };
            self.next_comet = (self.next_comet + 1) % COMETS;
        }

        for ring in self.rings.iter_mut().filter(|ring| ring.active) {
            ring.age += dt;
            if ring.age > 1.8 {
                ring.active = false;
            }
        }
        for comet in self.comets.iter_mut().filter(|comet| comet.active) {
            comet.age += dt;
            if comet.age > 2.4 {
                comet.active = false;
            }
        }

        let alive = self.rings.iter().any(|ring| ring.active)
            || self.comets.iter().any(|comet| comet.active);
        alive
            || self.level > QUIET
            || self.levels.iter().any(|level| *level > QUIET)
            || self.swell > 0.01
    }

    /// The ridge height at `t` (0 to 1 across the width), in fractions of the field's height.
    pub(crate) fn ridge(&self, stratum: usize, t: f32, amplitude: f32) -> f32 {
        let phase = self.phases[stratum];
        let cycles = 1.0 + 0.15 * stratum as f32;
        let wave = (t * cycles + phase) * std::f32::consts::TAU;
        let swell = (t * cycles * 0.5 - phase * 0.7 + stratum as f32) * std::f32::consts::TAU;
        amplitude * (wave.sin() * 0.82 + swell.sin() * 0.18)
    }

    /// The floor a ridge hangs from, in fractions of the height.
    pub(crate) fn floor(stratum: usize) -> f32 {
        0.86 - 0.115 * stratum as f32
    }

    /// How far a ridge rises from its floor, in pixels.
    pub(crate) fn amplitude(&self, stratum: usize, height: f32) -> f32 {
        let wanted = height
            * (0.028 + 0.125 * self.levels[stratum] + 0.05 * self.swell)
            * (0.85 + 0.05 * stratum as f32);
        wanted.min(height * Self::floor(stratum) / 1.35)
    }

    /// Where the ridge's own centre line sits, in pixels from the top of the field.
    pub(crate) fn ridge_base(&self, stratum: usize, height: f32) -> f32 {
        Self::floor(stratum) * height - self.amplitude(stratum, height) * 0.35
    }
}
