//! Wo die Sonne im Laufe des Jahres auf den Boden faellt.
//!
//! R5 fragt bisher, ob der Schreibtisch mit dem Gesicht oder dem Ruecken zur Verglasung
//! steht — eine Aussage ueber die Achse, und keine ueber Licht. Sie kommt ohne Ort, ohne
//! Datum und ohne Uhrzeit aus, und deshalb kann sie nicht beantworten, was jemand am
//! Schreibtisch wirklich wissen will: *faellt mir im Maerz um neun die Sonne auf den
//! Bildschirm?*
//!
//! Diese Datei rechnet das. Zwei Schritte, beide gewoehnliche Geometrie:
//!
//! 1. **Wo steht die Sonne** — Azimut und Hoehe aus Datum, Uhrzeit und Ort, nach dem Verfahren
//!    der NOAA. Reine Trigonometrie, keine Tabelle, keine Naeherung ueber Mittelwerte.
//! 2. **Wohin faellt ihr Licht** — die Verglasung ist eine senkrechte Flaeche zwischen zwei
//!    Hoehen; das Licht durch sie zeichnet auf dem Boden ein Parallelogramm, dessen Versatz
//!    `hoehe / tan(sonnenhoehe)` betraegt. Steht ein Moebel darin, liegt es in der Sonne.
//!
//! ## Was die Wohnung dafuer sagen muss, und was passiert, wenn sie schweigt
//!
//! ```toml
//! [lage]
//! breite = 50.7            # Grad Nord
//! laenge = 7.1             # Grad Ost
//! utc_offset_h = 2         # die Zeitzone MIT Sommerzeit, wenn die Uhrzeiten Sommerzeit sind
//! nordrichtung_grad = 0    # welche Kompassrichtung im Plan nach oben zeigt
//!
//! [[oeffnungen]]
//! id = "terrassentuer"
//! glas_von_cm = 0          # Unterkante ueber dem Boden
//! glas_bis_cm = 210        # Oberkante
//! ```
//!
//! Fehlt `[lage]`, wird **nicht gerechnet**. Ein erfundener Standort ergaebe einen Schattenwurf
//! auf den Zentimeter genau, der auf nichts beruht — das waere schlimmer als keine Antwort.
//! Dasselbe gilt fuer eine Verglasung ohne Hoehen: die Ausdehnung des Lichtflecks haengt
//! linear an ihnen. Beides meldet der Bericht als Luecke, wie es `CheckResult::nicht_geprueft`
//! seit PRD B31 fuer jede nicht messbare Regel tut.
//!
//! ## Wo dieses Modell zu grob ist, und in welche Richtung
//!
//! Der Lichtfleck ist der **ungehinderte** Wurf: was zwischen Fenster und Boden steht, wirft
//! keinen Schatten. Damit meldet die Rechnung eher zu viel Sonne als zu wenig — die
//! vorsichtige Richtung fuer eine Blendungsfrage. Nachbarhaeuser, Baeume und Balkonplatten
//! kennt sie nicht; ein Nordbalkon ueber der Terrassentuer wuerde jede Zahl hier senken.

use crate::model::{Model, ModelError, Opening, Pt, Rect};
use serde::Serialize;

/// Wo die Sonne steht.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SunPosition {
    /// Grad ueber dem Horizont. Negativ heisst: unter dem Horizont.
    pub altitude_deg: f64,
    /// Kompassgrad, von Nord im Uhrzeigersinn.
    pub azimuth_deg: f64,
}

/// Sonnenstand nach dem Verfahren der NOAA (General Solar Position Calculations).
///
/// `tag_im_jahr` und `stunde_utc` statt eines Datumstyps: diese Capability haengt an keiner
/// Kalenderbibliothek, und die Frage hier ist eine astronomische, keine kalendarische.
/// `jahr` geht nur ueber die Julianische Tageszahl ein.
pub fn sun_position(
    year: i32,
    day_of_year: u32,
    hour_utc: f64,
    width: f64,
    length: f64,
) -> SunPosition {
    // Julianische Tageszahl fuer den Jahresanfang, plus der Tag. Genauigkeit im Bereich von
    // Sekunden reicht: gefragt ist, ob die Sonne auf einen Schreibtisch faellt.
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

    // Zeitgleichung: der Unterschied zwischen wahrer und mittlerer Sonnenzeit, in Minuten.
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

    // Azimut aus dem Zenitwinkel. Der Nenner wird null, wenn die Sonne im Zenit steht; dann
    // ist der Azimut nicht definiert und jede Antwort gleich richtig.
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

/// Der Lichtfleck einer Verglasung auf dem Boden: ein Parallelogramm in Raumkoordinaten.
#[derive(Debug, Clone, Serialize)]
pub struct LightPatch {
    pub opening: String,
    /// Vier Ecken, gegen den Uhrzeigersinn oder mit — fuer den Ueberschneidungstest egal.
    pub corners: [[f64; 2]; 4],
}

/// Die Richtung, in die das Licht laeuft, in Plankoordinaten.
///
/// `azimut` ist ein Kompasswinkel; der Plan haengt ueber `nordrichtung_grad` daran. Das Licht
/// laeuft von der Sonne WEG, also in die Gegenrichtung ihres Azimuts.
fn light_direction(azimuth_deg: f64, north_deg: f64) -> [f64; 2] {
    let b = (azimuth_deg + 180.0 - north_deg).to_radians();
    // Plan: +x nach Osten, +y nach Sueden. Norden ist also -y.
    [b.sin(), -b.cos()]
}

/// Wohin das Licht durch diese Oeffnung faellt — oder `None`, wenn es das nicht tut.
///
/// Drei Gruende fuer `None`, und keiner davon ist ein Fehler: die Sonne steht unter dem
/// Horizont, sie steht hinter der Wand, oder die Oeffnung sagt ihre Glashoehen nicht.
pub fn light_patch(
    model: &Model,
    o: &Opening,
    stand: SunPosition,
    north_deg: f64,
) -> Option<LightPatch> {
    let (from_cm, bis_cm) = (o.glass_from_cm?, o.glass_to_cm?);
    if stand.altitude_deg <= 0.5 {
        // Unter einem halben Grad ist der Versatz groesser als jede Wohnung, und die Rechnung
        // liefe gegen unendlich statt gegen eine Aussage.
        return None;
    }
    let seg = crate::clearance::opening_segment(&model.room, o)?;
    let d = light_direction(stand.azimuth_deg, north_deg);
    // Scheint sie ueberhaupt auf diese Fassade? Das Licht muss nach innen laufen.
    if d[0] * seg.normal[0] + d[1] * seg.normal[1] <= 0.0 {
        return None;
    }
    let tan_h = stand.altitude_deg.to_radians().tan();
    let offset = |z: f64| z / tan_h;
    let (near, fern) = (offset(bis_cm as f64), offset(from_cm as f64));
    // Die OBERkante wirft am weitesten? Nein — sie steht am hoechsten, also faellt ihr Strahl
    // am weitesten in den Raum. Die Unterkante zeichnet den nahen Rand.
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

/// Ueberschneiden sich ein Rechteck und ein Parallelogramm?
///
/// Trennachsensatz ueber die Kantennormalen beider Formen. Beide sind konvex, also ist eine
/// trennende Achse gleichbedeutend mit „beruehren sich nicht" — kein Naeherungsverfahren,
/// sondern ein Beweis in beide Richtungen.
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

/// Ein geprueftes Datum mit seiner Uhrzeit.
#[derive(Debug, Clone, Serialize)]
pub struct SunHour {
    pub tag: &'static str,
    pub hour_local: u32,
    pub altitude_deg: f64,
    pub azimuth_deg: f64,
    /// Welche Stuecke in diesem Moment im direkten Licht liegen.
    pub hit: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SunReport {
    pub layout: String,
    pub hours: Vec<SunHour>,
    /// Je Stueck: in wie vielen der geprueften Stunden es im direkten Licht liegt.
    pub hits_per_piece: std::collections::BTreeMap<String, usize>,
    /// Verglasungen, deren Hoehen fehlen — sie werfen hier kein Licht und muessten es.
    pub without_glass_heights: Vec<String>,
}

/// Die vier Tage, an denen sich das Jahr entscheidet.
///
/// Wintersonnenwende und Sommersonnenwende sind die Extreme, die Tagundnachtgleichen der
/// Mittelwert. Wer diese vier kennt, kennt die Spanne — jeder weitere Tag liegt dazwischen.
/// Die Tagesnummern sind fuer ein Gemeinjahr und koennen im Schaltjahr um einen Tag danebe
/// liegen; auf den Sonnenstand wirkt sich das um Bruchteile eines Grades aus.
pub const DAYS: &[(&str, u32)] = &[
    ("21. Maerz", 80),
    ("21. Juni", 172),
    ("23. September", 266),
    ("21. Dezember", 355),
];

/// Von wann bis wann geprueft wird, in lokaler Uhrzeit.
pub const HOURS: std::ops::RangeInclusive<u32> = 8..=18;

/// Das laufende Kalenderjahr, aus der Uhr statt aus dem Quelltext.
///
/// Bis 2026-08-31 stand hier die Zahl 2026. Ein Bericht, der jedes Jahr denselben Sonnenstand
/// rechnet, laeuft mit dem Kalender auseinander — und zwar lautlos, weil seine Zahlen weiter
/// auf die Kommastelle genau aussehen.
///
/// Einmal ermittelt und behalten: `stunden_in_der_sonne` haengt in der Suchschleife ueber
/// Millionen Aufstellungen, und ein Systemaufruf je Stunde je Kandidat waere dort teuer. Ein
/// Prozess, der ueber den Jahreswechsel laeuft, rechnet also mit dem Jahr seines Starts weiter
/// — der Unterschied liegt bei Bruchteilen eines Grades, derselbe Bereich, in dem `TAGE` das
/// Schaltjahr ignoriert.
fn year_now() -> i32 {
    static YEAR: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *YEAR.get_or_init(|| {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        year_from_days(seconds.div_euclid(86_400))
    })
}

/// Tage seit 1970-01-01 zum Kalenderjahr, nach Howard Hinnants `civil_from_days`.
///
/// Von Hand gerechnet und nicht aus einer Kalenderbibliothek: fuer eine einzige Jahreszahl
/// waere das eine Abhaengigkeit zu viel, und `capabilities/transit/src/main.rs` traegt
/// dieselbe Rechnung aus demselben Grund.
fn year_from_days(days: i64) -> i32 {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    // Hinnants Jahr beginnt im Maerz, damit der Schalttag ans Ende faellt. Die beiden Monate
    // davor gehoeren deshalb schon ins naechste Kalenderjahr.
    let march_year = yoe + era * 400;
    (march_year + i64::from((5 * doy + 2) / 153 >= 10)) as i32
}

/// In wie vielen der geprueften Stunden liegt dieses Rechteck im direkten Licht?
///
/// Die schmale Fassung fuer `clearance.rs`: eine Regel braucht eine Zahl und keinen Bericht,
/// und sie wird in einer Schleife aufgerufen, die Millionen Aufstellungen prueft.
pub fn hours_in_sun(model: &Model, r: &Rect) -> Result<usize, ModelError> {
    let Some(location) = model.room.location else {
        return Err(ModelError::Missing(
            "room.toml fuehrt kein [lage] — ohne Ort gibt es keinen Sonnenstand".into(),
        ));
    };
    let mut n = 0;
    for (_, tag) in DAYS {
        for h in HOURS {
            let stand = sun_position(
                year_now(),
                *tag,
                h as f64 - location.utc_offset_h,
                location.width,
                location.length,
            );
            let hit = model
                .room
                .openings
                .iter()
                .filter(|o| {
                    o.typ
                        .as_deref()
                        .is_some_and(|t| t.starts_with("glastuer") || t.starts_with("fenster"))
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

/// Wie viele Stunden dieser Bericht ueberhaupt prueft. Der Nenner zu jeder Trefferzahl.
pub fn checked_hours() -> usize {
    DAYS.len() * HOURS.count()
}

/// Wann im Jahr welches Moebel in der Sonne steht.
pub fn bericht(model: &Model, layout: &crate::model::Layout) -> Result<SunReport, ModelError> {
    let location = model.room.location.ok_or_else(|| {
        ModelError::Missing(
            "room.toml fuehrt kein [lage] — ohne Breite, Laenge und Nordrichtung ist ein \
             Sonnenstand eine erfundene Zahl"
                .into(),
        )
    })?;

    let glazings: Vec<&Opening> = model
        .room
        .openings
        .iter()
        .filter(|o| {
            o.typ
                .as_deref()
                .is_some_and(|t| t.starts_with("glastuer") || t.starts_with("fenster"))
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
            let stand = sun_position(year_now(), *tag, utc, location.width, location.length);
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

    /// Feste Tageszahlen statt der Uhr: ein Test, der `jahr_jetzt()` gegen ein Literal haelt,
    /// waere genau der Fehler, den diese Aenderung behebt.
    #[test]
    fn the_year_boundary_is_on_the_right_day() {
        assert_eq!(year_from_days(0), 1970, "1970-01-01 ist Tag null");
        // 56 Jahre a 365 Tage plus 14 Schalttage (2000 zaehlt mit, es ist durch 400 teilbar).
        assert_eq!(year_from_days(20_454), 2026, "2026-01-01");
        assert_eq!(year_from_days(20_453), 2025, "der Tag davor");
        assert_eq!(year_from_days(-1), 1969, "und rueckwaerts kippt es auch");
    }

    /// Die Uhr liefert ueberhaupt ein Jahr aus diesem Jahrhundert.
    #[test]
    fn the_clock_gives_a_plausible_year() {
        let j = year_now();
        assert!((2020..2100).contains(&j), "{j}");
    }
}
