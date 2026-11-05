//! The window's visual: strata that move with the sound.
//!
//! This is the native answer to the dashboard's fluid sim, and it is deliberately not a port. That
//! one is a GPU fluid solver because it lives in a browser canvas and a page can afford a second
//! simulation next to its audio; this one is drawn with the GPU the window already has, out of the
//! spectrum the synth is *actually* playing. Seven ridges, each carrying a slice of the spectrum,
//! each drifting at its own rate, filled downward so they stack like silk. A kick swells them and
//! drops a ring; a note sends a bright point travelling along the ridge its pitch belongs to.
//!
//! Every number that moves here comes from [`Visual`], so the picture cannot get ahead of or behind
//! the sound: if the mix stops, the ridges settle, because their amplitude is the mix.

pub mod canvas;
pub mod constants;
pub mod field;
pub mod paint;
pub mod types;

pub use canvas::{canvas, frame};
pub use constants::{COMETS, POINTS, QUIET, STRATA};
pub use field::Field;

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::soundscape::{Preset, Visual, BANDS};

    fn field_with(bands: [f32; BANDS], rms: f32) -> (Field, Visual) {
        let visual = Visual::new();
        visual.write_spectrum(bands, rms, rms);
        let mut field = Field::new();
        // Settle: the field smooths everything, so one step is not a state.
        for _ in 0..120 {
            field.advance(&visual, Preset::Focus, 1. / 60.);
        }
        (field, visual)
    }

    /// The same gesture at two frame rates: a 120 Hz window and a 60 Hz one must agree, which is
    /// what the time-based smoothing is for. Per-frame constants made them differ by a factor of two.
    #[test]
    fn the_motion_does_not_depend_on_the_frame_rate() {
        let mut bands = [0f32; BANDS];
        bands[4] = 0.8;
        let visual = Visual::new();
        visual.write_spectrum(bands, 0.4, 0.4);

        let mut fast = Field::new();
        let mut slow = Field::new();
        for _ in 0..120 {
            fast.advance(&visual, Preset::Focus, 1. / 120.);
        }
        for _ in 0..60 {
            slow.advance(&visual, Preset::Focus, 1. / 60.);
        }
        let (a, b) = (fast.levels[4], slow.levels[4]);
        assert!((a - b).abs() < 0.02, "120 Hz settled at {a}, 60 Hz at {b}");
    }

    /// Silence has to settle: the field says so, and the window stops redrawing.
    #[test]
    fn a_quiet_field_reports_itself_still() {
        let (mut field, visual) = field_with([0.; BANDS], 0.);
        // It has been fed silence for two seconds and is still settling from its own start.
        for _ in 0..600 {
            field.advance(&visual, Preset::Focus, 1. / 60.);
        }
        assert!(!field.advance(&visual, Preset::Focus, 1. / 60.));
        assert!(field.levels.iter().all(|level| *level < QUIET));
    }

    /// A playing mix keeps it moving.
    #[test]
    fn a_sounding_field_reports_itself_moving() {
        let mut bands = [0f32; BANDS];
        bands[10] = 0.5;
        let (mut field, visual) = field_with(bands, 0.3);
        assert!(field.advance(&visual, Preset::Focus, 1. / 60.));
    }

    /// A kick is one ring and one swell, not one per frame: the counters are the whole reason the
    /// events arrive as counters instead of as a flag.
    #[test]
    fn one_kick_is_one_ring() {
        let (mut field, visual) = field_with([0.; BANDS], 0.);
        visual.write_events(1, 0, 60, 0.);
        field.advance(&visual, Preset::Focus, 1. / 60.);
        let rings = field.rings.iter().filter(|ring| ring.active).count();
        assert_eq!(rings, 1);
        // The same counter again is not a second kick.
        field.advance(&visual, Preset::Focus, 1. / 60.);
        assert_eq!(field.rings.iter().filter(|ring| ring.active).count(), 1);
        // A new count is.
        visual.write_events(1, 0, 60, 0.);
        field.advance(&visual, Preset::Focus, 1. / 60.);
        assert_eq!(field.rings.iter().filter(|ring| ring.active).count(), 2);
    }

    /// A note becomes a comet on the ridge its pitch belongs to.
    #[test]
    fn a_note_lands_on_its_own_ridge() {
        let (mut field, visual) = field_with([0.; BANDS], 0.);
        visual.write_events(0, 1, 72, -0.5);
        field.advance(&visual, Preset::Focus, 1. / 60.);
        let comet = field
            .comets
            .iter()
            .find(|comet| comet.active)
            .expect("a comet");
        assert_eq!(comet.stratum, (72i32.rem_euclid(12) as usize) * STRATA / 12);
        assert!(comet.from < 0.5, "a note panned left starts left");
    }

    /// The palette fades to the new preset rather than jumping to it.
    #[test]
    fn a_preset_change_fades_the_palette() {
        let mut field = Field::new();
        let visual = Visual::new();
        field.advance(&visual, Preset::Edm, 1. / 60.);
        let edm = field.palette;
        field.advance(&visual, Preset::Sleep, 1. / 60.);
        let just_after = field.palette;
        assert_ne!(just_after, edm, "the fade never started");
        assert_ne!(just_after, Preset::Sleep.palette(), "the palette jumped");
        for _ in 0..120 {
            field.advance(&visual, Preset::Sleep, 1. / 60.);
        }
        let settled = field.palette;
        let target = Preset::Sleep.palette();
        for (channel, wanted) in settled.iter().zip(target) {
            for (value, want) in channel.iter().zip(wanted) {
                assert!((value - want).abs() < 0.01, "{settled:?} vs {target:?}");
            }
        }
    }

    /// Ridges stay inside the field at any level, which is what keeps a loud passage from drawing
    /// off the top of the window.
    #[test]
    fn a_full_level_stays_inside_the_field() {
        let mut bands = [1f32; BANDS];
        bands[0] = 1.;
        let (mut field, visual) = field_with(bands, 1.);
        for _ in 0..60 {
            field.advance(&visual, Preset::Edm, 1. / 60.);
        }
        for stratum in 0..STRATA {
            let height = 1.0;
            let amplitude = field.amplitude(stratum, height);
            let base = field.ridge_base(stratum, height);
            for step in 0..=POINTS {
                let t = step as f32 / POINTS as f32;
                let top = base - field.ridge(stratum, t, amplitude);
                assert!(
                    top >= -0.001,
                    "ridge {stratum} drew above the field at {top}"
                );
                assert!(top <= 1.0, "ridge {stratum} drew below the field at {top}");
            }
        }
    }
}
