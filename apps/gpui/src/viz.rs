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
//!
//! Two rules hold the motion together, and both were learned by watching it go wrong:
//!
//! - **Time, not frames.** Every smoothing is `1 - exp(-dt / tau)`, so a 120 Hz display and a 60 Hz
//!   one show the same gesture. Per-frame constants made the same sound look twitchy on one and
//!   sluggish on the other.
//! - **Nothing is drawn when nothing moves.** The field reports whether it is still settling, and
//!   the window stops asking for frames when it is not: a paused soundscape is a still picture, not
//!   a busy loop.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{div, point, prelude::*, px, rgba, Bounds, IntoElement, Pixels, Window};
use sjel_gpui_core::soundscape::{Preset, Visual, BANDS};

/// Ridges drawn, bottom of the field upward. Seven is enough to read as layers without turning the
/// window into a bar chart.
const STRATA: usize = 7;
/// Points along one ridge. A path is tessellated per frame, so this is a cost as well as a shape.
const POINTS: usize = 96;
/// Comets alive at once; a note beyond this replaces the oldest.
const COMETS: usize = 10;
/// A ridge is at rest below this, and a level below it means the field is still.
const QUIET: f32 = 0.004;

/// One point of light travelling along a ridge.
#[derive(Clone, Copy, Debug, Default)]
struct Comet {
    active: bool,
    /// Seconds since it started.
    age: f32,
    /// Which ridge it rides.
    stratum: usize,
    /// Where it started, 0 (left) to 1 (right), and how far it travels.
    from: f32,
    span: f32,
    strength: f32,
}

/// A ring from a kick: the one thing in the picture that is an event rather than a level.
#[derive(Clone, Copy, Debug, Default)]
struct Ring {
    active: bool,
    age: f32,
    strength: f32,
}

/// The field's own state: what it has already reacted to, and how far each ridge has drifted.
pub struct Field {
    /// Amplitude per ridge, smoothed towards its band's own level.
    levels: [f32; STRATA],
    /// Drift phase per ridge, in turns.
    phases: [f32; STRATA],
    /// The swell a kick leaves behind, which is smoother than moving the ridges themselves.
    swell: f32,
    /// The palette actually drawn, moving towards the preset's own so a preset change is a fade
    /// rather than a jump.
    palette: [[f32; 3]; 4],
    palette_set: bool,
    comets: [Comet; COMETS],
    rings: [Ring; STRATA],
    /// The mix's level, smoothed for the rule at the top.
    level: f32,
    /// The event counters the last frame saw, so a kick is one ring and not one per frame.
    kicks_seen: u32,
    notes_seen: u32,
    next_comet: usize,
}

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

/// A value moving towards a target with a time constant, in seconds.
///
/// The whole reason the motion is smooth at any frame rate: `tau` is a duration, not a step.
fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if tau <= 0. {
        return target;
    }
    current + (target - current) * (1. - (-dt / tau).exp())
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
    ///
    /// One shape serves the fill, the edge and the comets, so a note rides the line it belongs to
    /// instead of a straight line near it.
    fn ridge(&self, stratum: usize, t: f32, amplitude: f32) -> f32 {
        let phase = self.phases[stratum];
        let cycles = 1.0 + 0.15 * stratum as f32;
        let wave = (t * cycles + phase) * std::f32::consts::TAU;
        let swell = (t * cycles * 0.5 - phase * 0.7 + stratum as f32) * std::f32::consts::TAU;
        amplitude * (wave.sin() * 0.82 + swell.sin() * 0.18)
    }

    /// The floor a ridge hangs from, in fractions of the height.
    fn floor(stratum: usize) -> f32 {
        0.86 - 0.115 * stratum as f32
    }

    /// How far a ridge rises from its floor, in pixels.
    ///
    /// Capped by the room above the floor: a band near the top of the field has less space to swell
    /// into, and without the cap the highest ridge drew past the window and was clipped flat, which
    /// looks like a broken picture rather than a loud passage.
    fn amplitude(&self, stratum: usize, height: f32) -> f32 {
        let wanted = height
            * (0.028 + 0.125 * self.levels[stratum] + 0.05 * self.swell)
            * (0.85 + 0.05 * stratum as f32);
        wanted.min(height * Self::floor(stratum) / 1.35)
    }

    /// Where the ridge's own centre line sits, in pixels from the top of the field.
    fn ridge_base(&self, stratum: usize, height: f32) -> f32 {
        Self::floor(stratum) * height - self.amplitude(stratum, height) * 0.35
    }

    /// Draw the field into `bounds`.
    pub fn paint(&self, visual: &Visual, bounds: Bounds<Pixels>, window: &mut Window) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width < 8. || height < 8. {
            return;
        }
        let origin = bounds.origin;
        let colours = self.palette;
        let at = |x: f32, y: f32| point(origin.x + px(x), origin.y + px(y));

        // The wash behind the ridges: the preset's own colours, top and bottom, so the field belongs
        // to the preset instead of being one dark rectangle for all six.
        window.paint_quad(gpui_kit::PaintQuad {
            bounds,
            corner_radii: gpui_kit::Corners::default(),
            background: gpui_kit::linear_gradient(
                180.,
                gpui_kit::linear_color_stop(rgba(rgb_alpha(colours[2], 0.26)), 0.),
                gpui_kit::linear_color_stop(rgba(rgb_alpha(colours[0], 0.09)), 1.),
            ),
            border_widths: gpui_kit::Edges::default(),
            border_color: rgba(0x00000000).into(),
            border_style: gpui_kit::BorderStyle::default(),
        });

        for stratum in 0..STRATA {
            let level = self.levels[stratum];
            let floor = Self::floor(stratum);
            // The swell adds on a kick without changing the ridge's own level, so the beat is felt
            // and the spectrum is still what sets the size.
            let amplitude = height
                * (0.028 + 0.125 * level + 0.05 * self.swell)
                * (0.85 + 0.05 * stratum as f32);
            let colour = colours[stratum % colours.len()];
            let ridge_base = floor * height - amplitude * 0.35;
            let ridge = |t: f32| ridge_base - self.ridge(stratum, t, amplitude);

            let mut body = gpui_kit::PathBuilder::fill();
            body.move_to(at(0., floor * height));
            for step in 0..POINTS {
                let t = step as f32 / (POINTS - 1) as f32;
                body.line_to(at(width * t, ridge(t)));
            }
            body.line_to(at(width, floor * height));
            body.close();
            if let Ok(path) = body.build() {
                window.paint_path(path, rgba(rgb_alpha(colour, 0.07 + 0.15 * level)));
            }

            // The ridge itself, brighter than its body: the line is what the eye follows, and it is
            // the only part of the band that shows a quiet passage at all.
            let mut edge = gpui_kit::PathBuilder::stroke(px(1. + 1.1 * level));
            for step in 0..POINTS {
                let t = step as f32 / (POINTS - 1) as f32;
                let here = at(width * t, ridge(t));
                if step == 0 {
                    edge.move_to(here);
                } else {
                    edge.line_to(here);
                }
            }
            if let Ok(path) = edge.build() {
                window.paint_path(path, rgba(rgb_alpha(colour, 0.26 + 0.5 * level)));
            }
        }

        // A ring per kick: two concentric ellipses near the bottom, which is where a kick is felt.
        // Eased out and thinned as it goes, so it reads as something moving away.
        for ring in self.rings.iter().filter(|ring| ring.active) {
            let progress = (ring.age / 1.8).clamp(0., 1.);
            let eased = 1. - (1. - progress) * (1. - progress);
            let radius = width * (0.05 + 0.30 * eased);
            let fade = (1. - progress).powi(2) * ring.strength;
            let centre = point(origin.x + px(width * 0.5), origin.y + px(height * 0.8));
            for (scale, alpha) in [(1.0, 0.30), (0.86, 0.16)] {
                let mut builder =
                    gpui_kit::PathBuilder::stroke(px((0.8 + 1.6 * (1. - progress)) * scale));
                let steps = 48;
                for step in 0..=steps {
                    let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
                    let x = centre.x + px(radius * scale * angle.cos());
                    // Flattened vertically: a circle in a landscape window reads as a bubble, an
                    // ellipse reads as something seen from the side.
                    let y = centre.y + px(radius * scale * angle.sin() * 0.3);
                    if step == 0 {
                        builder.move_to(point(x, y));
                    } else {
                        builder.line_to(point(x, y));
                    }
                }
                if let Ok(path) = builder.build() {
                    window.paint_path(path, rgba(rgb_alpha(colours[1], alpha * fade)));
                }
            }
        }

        // A comet per note: a short trail of dots travelling along its ridge, fading in as it
        // arrives and out as it goes.
        for comet in self.comets.iter().filter(|comet| comet.active) {
            let progress = (comet.age / 2.4).clamp(0., 1.);
            let stratum = comet.stratum.min(STRATA - 1);
            let amplitude = self.amplitude(stratum, height);
            let ridge_base = self.ridge_base(stratum, height);
            // A soft arrival and a long goodbye, which is what keeps a run of notes from flickering.
            let fade = (progress / 0.12).min(1.) * (1. - progress).powf(1.4) * comet.strength;
            let colour = colours[(stratum + 1) % colours.len()];
            for (trail, scale) in [(0., 1.0), (0.018, 0.62), (0.036, 0.34)] {
                let t = comet.from + comet.span * (progress - trail);
                if !(0. ..=1.).contains(&t) {
                    continue;
                }
                let size = px((2.2 + 2.4 * (1. - progress)) * scale);
                let y = ridge_base - self.ridge(stratum, t, amplitude);
                let dot = Bounds {
                    origin: at(width * t - f32::from(size) * 0.5, y - f32::from(size) * 0.5),
                    size: gpui_kit::size(size, size),
                };
                window.paint_quad(gpui_kit::PaintQuad {
                    bounds: dot,
                    corner_radii: gpui_kit::Corners::all(size * 0.5),
                    background: rgba(rgb_alpha(colour, 0.75 * fade)).into(),
                    border_widths: gpui_kit::Edges::default(),
                    border_color: rgba(0x00000000).into(),
                    border_style: gpui_kit::BorderStyle::default(),
                });
            }
        }

        // The level rule across the top: the mix's own level as a thin line, the one number that
        // says "there is sound" without any interpretation.
        let level = self.level.clamp(0., 1.);
        let span = width * level.sqrt();
        for (inset, alpha) in [(0., 0.5), (1., 0.16)] {
            let rule = Bounds {
                origin: point(origin.x, origin.y + px(inset)),
                size: gpui_kit::size(px(span), px(1.5)),
            };
            window.paint_quad(gpui_kit::PaintQuad {
                bounds: rule,
                corner_radii: gpui_kit::Corners::default(),
                background: rgba(rgb_alpha(colours[3], alpha)).into(),
                border_widths: gpui_kit::Edges::default(),
                border_color: rgba(0x00000000).into(),
                border_style: gpui_kit::BorderStyle::default(),
            });
        }

        let _ = visual;
    }
}

/// A palette colour at an alpha. The palette is `[r, g, b]` in `0.0..=1.0`, the same numbers the
/// browser engine uses, so this is the one place the two surfaces are held to one definition.
fn rgb_alpha(colour: [f32; 3], alpha: f32) -> u32 {
    let channel = |value: f32| ((value.clamp(0., 1.) * 255.).round() as u32) & 0xff;
    let alpha = ((alpha.clamp(0., 1.) * 255.).round() as u32) & 0xff;
    (channel(colour[0]) << 24) | (channel(colour[1]) << 16) | (channel(colour[2]) << 8) | alpha
}

/// The field, and the canvas element that paints it.
///
/// The size is explicit because a canvas has no content of its own: without it the element is laid
/// out at zero and the paint callback receives an empty rectangle, which looks exactly like a
/// visual that does not work.
pub fn canvas(field: Rc<RefCell<Field>>, visual: Arc<Visual>) -> impl IntoElement {
    gpui_kit::canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            field.borrow().paint(&visual, bounds, window);
        },
    )
    .w_full()
    .h_full()
}

/// The field's frame, so the window can hand it a bounded area.
pub fn frame(child: impl IntoElement) -> impl IntoElement {
    div().w_full().h_full().overflow_hidden().child(child)
}

#[cfg(test)]
mod tests {
    use super::*;

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
