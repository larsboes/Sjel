//! Canvas rendering pipeline for background washes, strata silk fills, edges, rings, and comets.

use gpui_kit::{point, px, rgba, Bounds, Pixels, Window};
use sjel_gpui_core::soundscape::Visual;

use super::constants::{POINTS, STRATA};
use super::field::Field;

impl Field {
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
pub(crate) fn rgb_alpha(colour: [f32; 3], alpha: f32) -> u32 {
    let channel = |value: f32| ((value.clamp(0., 1.) * 255.).round() as u32) & 0xff;
    let alpha = ((alpha.clamp(0., 1.) * 255.).round() as u32) & 0xff;
    (channel(colour[0]) << 24) | (channel(colour[1]) << 16) | (channel(colour[2]) << 8) | alpha
}
