//! Where sun falls on the floor through the year.
//!
//! Two steps, both plain geometry:
//!
//! 1. Where the sun stands: azimuth and altitude from date, time and place, after the NOAA method.
//! 2. Where its light falls: glazing is a vertical surface between two heights, and the light
//!    through it draws a parallelogram on the floor, offset by `height / tan(altitude)`. A piece
//!    inside it lies in the sun.
//!
//! What the flat must state:
//!
//! ```toml
//! [lage]                   # or [location]
//! breite = 50.7            # latitude, degrees north
//! laenge = 7.1             # longitude, degrees east
//! utc_offset_h = 2         # including daylight saving time if the hours are summer time
//! nordrichtung_grad = 0    # which compass direction points up in the plan
//!
//! [[oeffnungen]]
//! id = "terrassentuer"
//! glas_von_cm = 0          # lower edge above the floor
//! glas_bis_cm = 210        # upper edge
//! ```
//!
//! Without the location nothing is computed, and glazing without heights throws no light; the
//! report names both as gaps. The patch is the unobstructed throw: nothing between window and floor
//! casts a shadow, so the result errs towards too much sun, the safe side for glare. Neighbouring
//! buildings, trees and balconies are not modelled.

use crate::model::{Model, ModelError, Opening, Pt, Rect};
use serde::Serialize;

/// Where the sun stands.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SunPosition {
    /// Degrees above the horizon; negative is below it.
    pub altitude_deg: f64,
    /// Compass degrees, clockwise from north.
    pub azimuth_deg: f64,
}

/// Sun position after the NOAA General Solar Position Calculations. Day of year and UTC hour rather
/// than a date type: the question is astronomical, and the year enters only through the Julian day.
pub fn sun_position(
    year: i32,
    day_of_year: u32,
    hour_utc: f64,
    width: f64,
    length: f64,
) -> SunPosition {
    // Julian day of the year start plus the day. Seconds of accuracy are enough here.
    let a = ((14 - 1) / 12) as f64;
    let y = year as f64 + 4800.0 - a;
    let m = 1.0 + 12.0 * a - 3.0;
    let jd_year_start = 1.0 + (153.0 * m + 2.0).div_euclid(5.0) + 365.0 * y + (y / 4.0).floor()
        - (y / 100.0).floor()
        + (y / 400.0).floor()
        - 32045.0;
    let jd = jd_year_start + (day_of_year as f64 - 1.0) + (hour_utc - 12.0) / 24.0;

    let t = (jd - 2451545.0) / 36525.0;
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m_anom = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let m_rad = m_anom.to_radians();
    let c = (1.914602 - t * (0.004817 + 0.000014 * t)) * m_rad.sin()
        + (0.019993 - 0.000101 * t) * (2.0 * m_rad).sin()
        + 0.000289 * (3.0 * m_rad).sin();
    let true_longitude = l0 + c;
    let omega = 125.04 - 1934.136 * t;
    let lambda = true_longitude - 0.00569 - 0.00478 * omega.to_radians().sin();
    let eps0 = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let eps = eps0 + 0.00256 * omega.to_radians().cos();

    let declination = (eps.to_radians().sin() * lambda.to_radians().sin())
        .asin()
        .to_degrees();

    // Equation of time: true minus mean solar time, in minutes.
    let var_y = (eps.to_radians() / 2.0).tan().powi(2);
    let equation_of_time = 4.0
        * (var_y * (2.0 * l0.to_radians()).sin() - 2.0 * e * m_rad.sin()
            + 4.0 * e * var_y * m_rad.sin() * (2.0 * l0.to_radians()).cos()
            - 0.5 * var_y * var_y * (4.0 * l0.to_radians()).sin()
            - 1.25 * e * e * (2.0 * m_rad).sin())
        .to_degrees();

    let true_solar_time = (hour_utc * 60.0 + equation_of_time + 4.0 * length).rem_euclid(1440.0);
    let hour_angle = if true_solar_time / 4.0 < 0.0 {
        true_solar_time / 4.0 + 180.0
    } else {
        true_solar_time / 4.0 - 180.0
    };

    let (b, d, h) = (
        width.to_radians(),
        declination.to_radians(),
        hour_angle.to_radians(),
    );
    let zenith = (b.sin() * d.sin() + b.cos() * d.cos() * h.cos())
        .clamp(-1.0, 1.0)
        .acos();
    let height = 90.0 - zenith.to_degrees();

    // Azimuth from the zenith angle. The denominator is zero with the sun at the zenith, where the
    // azimuth is undefined and any answer is right.
    let denominator = b.cos() * zenith.sin();
    let azimuth = if denominator.abs() < 1e-9 {
        180.0
    } else {
        let cos_az = ((b.sin() * zenith.cos()) - d.sin()) / denominator;
        let az = cos_az.clamp(-1.0, 1.0).acos().to_degrees();
        if hour_angle > 0.0 {
            (az + 180.0).rem_euclid(360.0)
        } else {
            (540.0 - az).rem_euclid(360.0)
        }
    };

    SunPosition {
        altitude_deg: height,
        azimuth_deg: azimuth,
    }
}

/// A glazing's light patch on the floor: a parallelogram in room coordinates.
#[derive(Debug, Clone, Serialize)]
pub struct LightPatch {
    pub opening: String,
    /// Four corners, either winding; the overlap test does not care.
    pub corners: [[f64; 2]; 4],
}

/// The direction the light travels in plan coordinates: away from the sun, so opposite its
/// azimuth, rotated by `north_deg`.
fn light_direction(azimuth_deg: f64, north_deg: f64) -> [f64; 2] {
    let b = (azimuth_deg + 180.0 - north_deg).to_radians();
    // Plan: +x east, +y south, so north is -y.
    [b.sin(), -b.cos()]
}

/// Where light falls through this opening, or `None`: the sun is below the horizon, behind the
/// wall, or the opening states no glass heights.
pub fn light_patch(
    model: &Model,
    o: &Opening,
    stand: SunPosition,
    north_deg: f64,
) -> Option<LightPatch> {
    let (from_cm, bis_cm) = (o.glass_from_cm?, o.glass_to_cm?);
    if stand.altitude_deg <= 0.5 {
        // Below half a degree the offset exceeds any flat and runs towards infinity.
        return None;
    }
    let seg = crate::clearance::opening_segment(&model.room, o)?;
    let d = light_direction(stand.azimuth_deg, north_deg);
    // Does it shine on this facade at all? The light must travel inwards.
    if d[0] * seg.normal[0] + d[1] * seg.normal[1] <= 0.0 {
        return None;
    }
    let tan_h = stand.altitude_deg.to_radians().tan();
    let offset = |z: f64| z / tan_h;
    let (near, fern) = (offset(bis_cm as f64), offset(from_cm as f64));
    // The upper edge stands highest, so its ray reaches furthest into the room; the lower edge draws
    // the near border.
    let point = |p: Pt, w: f64| [p[0] as f64 + d[0] * w, p[1] as f64 + d[1] * w];
    Some(LightPatch {
        opening: o.id.clone(),
        corners: [
            point(seg.a, fern),
            point(seg.b, fern),
            point(seg.b, near),
            point(seg.a, near),
        ],
    })
}

/// Do a rect and a parallelogram overlap? Separating axis theorem over both shapes' edge normals;
/// both are convex, so this is exact in both directions.
pub fn touch(r: &Rect, l: &LightPatch) -> bool {
    let rect = [
        [r.x as f64, r.y as f64],
        [r.right() as f64, r.y as f64],
        [r.right() as f64, r.bottom() as f64],
        [r.x as f64, r.bottom() as f64],
    ];
    let axes = |p: &[[f64; 2]]| -> Vec<[f64; 2]> {
        (0..p.len())
            .map(|i| {
                let q = p[(i + 1) % p.len()];
                [-(q[1] - p[i][1]), q[0] - p[i][0]]
            })
            .collect()
    };
    for axis in axes(&rect).into_iter().chain(axes(&l.corners)) {
        let length = (axis[0] * axis[0] + axis[1] * axis[1]).sqrt();
        if length < 1e-9 {
            continue;
        }
        let proj = |p: &[[f64; 2]]| {
            p.iter().fold((f64::MAX, f64::MIN), |(lo, hi), q| {
                let v = (q[0] * axis[0] + q[1] * axis[1]) / length;
                (lo.min(v), hi.max(v))
            })
        };
        let (a0, a1) = proj(&rect);
        let (b0, b1) = proj(&l.corners);
        if a1 < b0 || b1 < a0 {
            return false;
        }
    }
    true
}

/// A checked date with its hour.
#[derive(Debug, Clone, Serialize)]
pub struct SunHour {
    pub tag: &'static str,
    pub hour_local: u32,
    pub altitude_deg: f64,
    pub azimuth_deg: f64,
    /// Which pieces lie in direct light at this moment.
    pub hit: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SunReport {
    pub layout: String,
    pub hours: Vec<SunHour>,
    /// Per piece: in how many checked hours it lies in direct light.
    pub hits_per_piece: std::collections::BTreeMap<String, usize>,
    /// Glazings without heights: they throw no light here and should.
    pub without_glass_heights: Vec<String>,
}

/// The four days that decide the year: the solstices are the extremes, the equinoxes the mean.
/// Day numbers are for a common year; in a leap year the sun moves a fraction of a degree.
pub const DAYS: &[(&str, u32)] = &[
    ("21 March", 80),
    ("21 June", 172),
    ("23 September", 266),
    ("21 December", 355),
];

/// The checked hours, local time.
pub const HOURS: std::ops::RangeInclusive<u32> = 8..=18;

/// The current calendar year, from the clock. Read once and kept: `hours_in_sun` runs inside a
/// search over millions of arrangements. A process running across New Year keeps its start year,
/// a difference of fractions of a degree.
fn year_now() -> i32 {
    static YEAR: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *YEAR.get_or_init(|| {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        year_from_days(seconds.div_euclid(86_400))
    })
}

/// Days since 1970-01-01 to the calendar year, after Howard Hinnant's `civil_from_days`, to avoid a
/// calendar dependency for one number.
fn year_from_days(days: i64) -> i32 {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    // Hinnant's year starts in March, so the two months before belong to the next calendar year.
    let march_year = yoe + era * 400;
    (march_year + i64::from((5 * doy + 2) / 153 >= 10)) as i32
}

/// In how many checked hours does this rect lie in direct light? The lean version for
/// `clearance.rs`, called in a loop over millions of arrangements.
pub fn hours_in_sun(model: &Model, r: &Rect) -> Result<usize, ModelError> {
    let Some(location) = model.room.location else {
        return Err(ModelError::Missing(
            "room.toml has no [lage]; without a location there is no sun position".into(),
        ));
    };
    let mut n = 0;
    for (_, tag) in DAYS {
        for h in HOURS {
            let stand = sun_position(
                year_now(),
                *tag,
                h as f64 - location.utc_offset_h,
                location.latitude,
                location.longitude,
            );
            let hit = model
                .room
                .openings
                .iter()
                .filter(|o| {
                    o.kind
                        .as_deref()
                        .is_some_and(|t| t.starts_with("glass_door") || t.starts_with("window"))
                })
                .filter_map(|o| light_patch(model, o, stand, location.north_deg))
                .any(|f| touch(r, &f));
            if hit {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// How many hours the report checks: the denominator of every hit count.
pub fn checked_hours() -> usize {
    DAYS.len() * HOURS.count()
}

/// When in the year which piece stands in the sun.
pub fn report(model: &Model, layout: &crate::model::Layout) -> Result<SunReport, ModelError> {
    let location = model.room.location.ok_or_else(|| {
        ModelError::Missing(
            "room.toml has no [lage]; without latitude, longitude and north direction a sun position is \
             an invented number"
                .into(),
        )
    })?;

    let glazings: Vec<&Opening> = model
        .room
        .openings
        .iter()
        .filter(|o| {
            o.kind
                .as_deref()
                .is_some_and(|t| t.starts_with("glass_door") || t.starts_with("window"))
        })
        .collect();
    let without_glass_heights: Vec<String> = glazings
        .iter()
        .filter(|o| o.glass_from_cm.is_none() || o.glass_to_cm.is_none())
        .map(|o| o.id.clone())
        .collect();

    let mut hours = Vec::new();
    let mut hits_per_piece: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for (name, tag) in DAYS {
        for h in HOURS {
            let utc = h as f64 - location.utc_offset_h;
            let stand = sun_position(year_now(), *tag, utc, location.latitude, location.longitude);
            let flecken: Vec<LightPatch> = glazings
                .iter()
                .filter_map(|o| light_patch(model, o, stand, location.north_deg))
                .collect();
            let mut hit = Vec::new();
            for it in &layout.items {
                let (w, d, _) = crate::model::footprint(it, &model.catalogue)?;
                let r = Rect {
                    x: it.x,
                    y: it.y,
                    w,
                    d,
                };
                if flecken.iter().any(|f| touch(&r, f)) {
                    hit.push(it.reference.clone());
                    *hits_per_piece.entry(it.reference.clone()).or_insert(0) += 1;
                }
            }
            hours.push(SunHour {
                tag: name,
                hour_local: h,
                altitude_deg: stand.altitude_deg,
                azimuth_deg: stand.azimuth_deg,
                hit,
            });
        }
    }

    Ok(SunReport {
        layout: layout.name.clone(),
        hours,
        hits_per_piece,
        without_glass_heights,
    })
}

#[cfg(test)]
mod year_tests {
    use super::{year_now, year_from_days};

    /// Fixed day numbers instead of the clock: a test holding `year_now()` against a literal would be
    /// the bug this fixes.
    #[test]
    fn the_year_boundary_is_on_the_right_day() {
        assert_eq!(year_from_days(0), 1970, "1970-01-01 is day zero");
        // 56 years of 365 days plus 14 leap days (2000 counts: divisible by 400).
        assert_eq!(year_from_days(20_454), 2026, "2026-01-01");
        assert_eq!(year_from_days(20_453), 2025, "the day before");
        assert_eq!(year_from_days(-1), 1969, "and it tips backwards too");
    }

    /// The clock gives a year of this century.
    #[test]
    fn the_clock_gives_a_plausible_year() {
        let j = year_now();
        assert!((2020..2100).contains(&j), "{j}");
    }
}
