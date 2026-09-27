//! Zeichnet ein Layout als SVG.
//!
//! Nichts hier kennt ein Mass. Jede Zahl auf dem Blatt kommt zur Laufzeit aus dem Modell, und
//! die Zeichnung entsteht aus denselben Koordinaten, gegen die der Pruefer urteilt — Bild und
//! Verdikt koennen deshalb nicht auseinanderlaufen. Genau diese Drift ist der Grund, warum es
//! keine handgezeichnete Planseite gibt.
//!
//! Selbsttragend: keine externe Schrift, kein Skript, kein Bildhost. Das SVG funktioniert aus
//! einer Datei, im Browser und in einer Vorschau ohne Netz.

use crate::clearance::{check_layout, kind_of, Kind};
use crate::model::{footprint, Layout, Model, ModelError, PlacedItem, Rect};

const PAD: i32 = 40;

/// Schriftgrade der Moebelbeschriftung, gross zuerst. Die viewBox laeuft in Zentimetern, also
/// IST ein Schriftgrad hier ein Mass auf dem Blatt und keine Bildschirmgroesse: er waechst mit
/// dem Plan und nicht mit dem Fenster.
const GRADE: [i32; 4] = [19, 16, 14, 12];

/// Mittlere Glyphenbreite von Helvetica als Anteil des Schriftgrads. GESCHAETZT und nicht
/// gemessen — das SVG traegt keine Schrift mit sich, also kann hier niemand die wirkliche
/// Breite kennen. Eher zu gross gewaehlt: ein zu klein geschaetzter Text laeuft ueber seine
/// Kante, ein zu gross geschaetzter kostet nur einen Schriftgrad.
const GLYPHE: f64 = 0.58;

/// Luft zwischen Text und Moebelkante.
const LUFT: i32 = 4;

/// Abstand zwischen Moebelkante und einer aussen gesetzten Beschriftung.
const ABSTAND: i32 = 8;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Kopfwoerter, an denen ein Bezeichner erkannt wird — geprueft als ENDUNG, weil ein deutsches
/// Kompositum sein Kopfwort hinten traegt: `buerostuhl` und `schreibtischstuhl` sind beide ein
/// Stuhl und enden beide auf `stuhl`. Es gewinnt der laengste Treffer, also faellt `couchtisch`
/// nicht auf `tisch` zurueck und die Reihenfolge dieser Liste traegt nichts. Anders als bei
/// `clearance::kind_of_name`, wo genau diese Reihenfolge einmal teuer war.
///
/// Die Liste ist eine Abkuerzung und keine Bedingung: ein Bezeichner, der hier nicht vorkommt,
/// bekommt sein eigenes Wort gross geschrieben und bleibt lesbar.
const KOPFWOERTER: &[(&str, &str)] = &[
    ("saugroboter", "Sauger"),
    ("beistelltisch", "Beistelltisch"),
    ("couchtisch", "Couchtisch"),
    ("schreibtisch", "Schreibtisch"),
    ("stuehle", "Stuhl"),
    ("stuhl", "Stuhl"),
    ("schrank", "Schrank"),
    ("pflanzen", "Pflanze"),
    ("pflanze", "Pflanze"),
    ("regale", "Regal"),
    ("regal", "Regal"),
    ("platte", "Platte"),
    ("tisch", "Tisch"),
    ("couch", "Couch"),
    ("sofa", "Couch"),
    ("bett", "Bett"),
];

/// Das Wort, das eine deklarierte Art auf dem Blatt traegt. `Other` hat keines — es ist die
/// Aussage "keine der bekannten Arten" und nicht der Name eines Moebels.
fn art_wort(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Bed => Some("Bett"),
        Kind::Desk => Some("Schreibtisch"),
        Kind::Couch => Some("Couch"),
        Kind::Wardrobe => Some("Schrank"),
        Kind::CoffeeTable => Some("Couchtisch"),
        Kind::Table => Some("Tisch"),
        Kind::Shelf => Some("Regal"),
        Kind::Other => None,
    }
}

fn gross(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Der kurze Name, der im Moebel steht.
///
/// Abgeleitet aus dem BEZEICHNER und nicht aus `label`. Der Katalogtext ist ein Satz fuer
/// Menschen — "2x Esstischstuhl", "Saugroboter mit Dock (Standardmass, kein Modell gewaehlt)" —
/// und stand bis 2026-08-31 nur an Komma und Klammer gekuerzt im Plan: beide Stuehle trugen
/// dadurch das Wort "2x", keiner passte in sein Rechteck, und die dritte Bodenpflanze deckte
/// die zweite zu. Der volle Name bleibt in der Oberflaeche am Stueck stehen, siehe
/// `dashboard/src/routes/interior/+page.svelte` (`focusRef` -> `it.label`).
///
/// Drei Schritte, jeder allgemein:
///
/// 1. Der Bezeichner zerfaellt an `_`. Das ERSTE Stueck ist das Wort, eine abschliessende Zahl
///    ist die laufende Nummer, alles dazwischen ist Beiwerk. `pflanze_boden_1` wird damit
///    "Pflanze 1" und `bett_bestand` wird "Bett", ohne eine Liste von Endungen zu pflegen.
/// 2. Das Wort wird auf sein Kopfwort gekuerzt, falls eines passt.
/// 3. Sonst zaehlt die im Layout DEKLARIERTE Art, und sonst das Wort selbst.
///
/// Diese Reihenfolge, weil der Name genauer ist als die Art: `clearance::kind_of_name` fuehrt
/// `buerostuhl` unter `Desk`, damit der Stuhl die Zonen eines Arbeitsplatzes erbt. Mit der Art
/// zuerst stand im Plan bis 2026-08-31 ein zweites "Schreibtisch" auf dem Buerostuhl — zwei
/// Kaesten, ein Wort, und eines davon gelogen. Die geerbte Art bleibt richtig; nur das WORT
/// kommt jetzt vom Stueck selbst.
pub fn short_label(reference: &str, deklariert: Option<Kind>) -> String {
    let mut teile: Vec<&str> = reference.split('_').filter(|s| !s.is_empty()).collect();
    let nummer = match teile.last() {
        Some(l) if teile.len() > 1 && l.chars().all(|c| c.is_ascii_digit()) => teile.pop(),
        _ => None,
    };
    let wort = teile
        .first()
        .copied()
        .unwrap_or(reference)
        .to_ascii_lowercase();
    let kopf = KOPFWOERTER
        .iter()
        .filter(|(endung, _)| wort.ends_with(endung))
        .max_by_key(|(endung, _)| endung.len())
        .map(|(_, kurz)| (*kurz).to_string())
        .or_else(|| deklariert.and_then(art_wort).map(str::to_string))
        .unwrap_or_else(|| gross(&wort));
    match nummer {
        Some(n) => format!("{kopf} {n}"),
        None => kopf,
    }
}

/// Wie breit ein Text auf dem Blatt wird. Siehe `GLYPHE`: eine Schaetzung, absichtlich zu gross.
fn breite(zeichen: usize, grad: i32) -> i32 {
    (zeichen as f64 * GLYPHE * grad as f64).ceil() as i32
}

/// Die Flaeche eines mittig gesetzten Textes. Fuer die Beschriftungen des RAUMS — Bad, Einbau,
/// Terrasse —, damit die Moebelbeschriftungen ihnen ausweichen statt sie zuzudecken. `sperrung`
/// ist das `letter-spacing`, das diese drei tragen und das die Schaetzung sonst unterschlaegt.
fn mittig(cx: i32, cy: i32, zeichen: usize, grad: i32, sperrung: i32, quer: bool) -> Rect {
    let lang = breite(zeichen, grad) + zeichen as i32 * sperrung;
    let (w, d) = if quer { (grad, lang) } else { (lang, grad) };
    Rect {
        x: cx - w / 2,
        y: cy - d / 2,
        w,
        d,
    }
}

/// Eine gesetzte Beschriftung: wo sie steht, wie gross, wie herum, und welche Flaeche sie damit
/// belegt. Die Flaeche ist der Punkt — sie ist es, gegen die die naechste Beschriftung prueft.
struct Schrift {
    x: i32,
    y: i32,
    grad: i32,
    anker: &'static str,
    quer: bool,
    fuehrung: Option<[i32; 4]>,
    flaeche: Rect,
}

fn frei(f: &Rect, hindernisse: &[Rect], gesetzt: &[Rect], blatt: &Rect) -> bool {
    f.x >= blatt.x
        && f.y >= blatt.y
        && f.right() <= blatt.right()
        && f.bottom() <= blatt.bottom()
        && !hindernisse.iter().any(|h| h.overlaps(f))
        && !gesetzt.iter().any(|g| g.overlaps(f))
}

/// Wohin die Beschriftung eines Moebels gehoert.
///
/// Zuerst hinein, so gross wie moeglich; passt sie liegend nicht, dann stehend — ein Rechteck,
/// das tiefer als breit ist, traegt seinen Text quer. Passt sie in keinem Schriftgrad hinein,
/// geht sie nach aussen, und dort wird die erste Stelle genommen, die weder ein Moebel noch
/// eine schon gesetzte Beschriftung noch den Blattrand trifft; eine Fuehrungslinie haelt sie an
/// ihrem Stueck. Erst wenn gar nichts frei ist, faellt sie klein nach Osten und darf
/// kollidieren — eine Beschriftung, die nirgends hinpasst, ist immer noch besser als keine, und
/// dass sie dann anstoesst, ist eine Aussage ueber die Enge.
///
/// Vorher entschied eine einzige Zeile — `zeichen * 11 > w && d > w` — ueber quer oder nicht und
/// liess den Text sonst laufen. Im Plan vom 2026-08-31 lagen dadurch zwei gedrehte Stuhlnamen
/// uebereinander, ein Pflanzenname war am Bildrand abgeschnitten, und der Name des Saugroboters
/// lag quer ueber drei anderen Stuecken.
///
/// `blatt` ist die ganze Zeichenflaeche, `kern` das Gemessene darin. Ein Name unter oder ueber
/// seinem Stueck wird in den Kern geschoben statt mittig ueber die Aussenwand zu rutschen —
/// sonst steht die Beschriftung eines Stuecks an der Fensterwand halb auf der Terrasse, also auf
/// einer geschaetzten Flaeche.
fn setze(
    r: &Rect,
    zeichen: usize,
    hindernisse: &[Rect],
    gesetzt: &[Rect],
    blatt: &Rect,
    kern: &Rect,
) -> Schrift {
    let (cx, cy) = (r.x + r.w / 2, r.y + r.d / 2);
    for grad in GRADE {
        let tw = breite(zeichen, grad);
        if tw + 2 * LUFT <= r.w && grad + 2 * LUFT <= r.d {
            return Schrift {
                x: cx,
                y: cy,
                grad,
                anker: "middle",
                quer: false,
                fuehrung: None,
                flaeche: Rect {
                    x: cx - tw / 2,
                    y: cy - grad / 2,
                    w: tw,
                    d: grad,
                },
            };
        }
        if tw + 2 * LUFT <= r.d && grad + 2 * LUFT <= r.w {
            return Schrift {
                x: cx,
                y: cy,
                grad,
                anker: "middle",
                quer: true,
                fuehrung: None,
                flaeche: Rect {
                    x: cx - grad / 2,
                    y: cy - tw / 2,
                    w: grad,
                    d: tw,
                },
            };
        }
    }

    let aussen = |grad: i32| -> [Schrift; 4] {
        let tw = breite(zeichen, grad);
        let halb = grad / 2;
        // Waagerecht in den Kern geschoben, solange der Text ueberhaupt hineinpasst.
        let laengs = if tw <= kern.w {
            (cx - tw / 2).clamp(kern.x, kern.right() - tw)
        } else {
            cx - tw / 2
        };
        [
            Schrift {
                x: r.right() + ABSTAND,
                y: cy,
                grad,
                anker: "start",
                quer: false,
                fuehrung: Some([r.right(), cy, r.right() + ABSTAND - 2, cy]),
                flaeche: Rect {
                    x: r.right() + ABSTAND,
                    y: cy - halb,
                    w: tw,
                    d: grad,
                },
            },
            Schrift {
                x: r.x - ABSTAND,
                y: cy,
                grad,
                anker: "end",
                quer: false,
                fuehrung: Some([r.x, cy, r.x - ABSTAND + 2, cy]),
                flaeche: Rect {
                    x: r.x - ABSTAND - tw,
                    y: cy - halb,
                    w: tw,
                    d: grad,
                },
            },
            Schrift {
                x: laengs + tw / 2,
                y: r.bottom() + ABSTAND + halb,
                grad,
                anker: "middle",
                quer: false,
                fuehrung: Some([cx, r.bottom(), cx, r.bottom() + ABSTAND - 2]),
                flaeche: Rect {
                    x: laengs,
                    y: r.bottom() + ABSTAND,
                    w: tw,
                    d: grad,
                },
            },
            Schrift {
                x: laengs + tw / 2,
                y: r.y - ABSTAND - halb,
                grad,
                anker: "middle",
                quer: false,
                fuehrung: Some([cx, r.y, cx, r.y - ABSTAND + 2]),
                flaeche: Rect {
                    x: laengs,
                    y: r.y - ABSTAND - grad,
                    w: tw,
                    d: grad,
                },
            },
        ]
    };

    for grad in GRADE {
        for kandidat in aussen(grad) {
            if frei(&kandidat.flaeche, hindernisse, gesetzt, blatt) {
                return kandidat;
            }
        }
    }
    let [osten, ..] = aussen(GRADE[GRADE.len() - 1]);
    osten
}

pub fn svg(model: &Model, layout: &Layout) -> Result<String, ModelError> {
    let room = &model.room;
    let poly = &room.hauptraum.polygon;
    // Nur GEMESSENES spannt das Blatt auf.
    //
    // Bis 2026-08-31 lief auch die Terrasse in die Ausdehnung ein. Sie ist geschaetzt — das
    // Wohnungsmodell fuehrt sie ausdruecklich so, ihre Tiefe ist aus der Luecke zwischen
    // Expose und Aufmass zurueckgerechnet und hat nie ein Bandmass gesehen — und sie nahm damit
    // ein knappes Viertel der Blattbreite. Der gemessene Raum stand auf der Haelfte des Blattes
    // und las sich kleiner, als er ist. Eine Schaetzung, die den Massstab des Gemessenen
    // bestimmt, ist genau die Falle, gegen die der Rest dieser Datei gebaut ist.
    //
    // Gezeichnet wird sie weiter, aber als Band, das der Blattrand ABSCHNEIDET: dass ihre Tiefe
    // unbekannt ist, steht damit im Bild und nicht in einer Fussnote.
    let mut xs: Vec<i32> = poly.iter().map(|p| p[0]).collect();
    let mut ys: Vec<i32> = poly.iter().map(|p| p[1]).collect();
    if let Some(b) = &room.bad {
        xs.extend(b.x);
        ys.extend(b.y);
    }
    let (min_x, max_x) = (
        xs.iter().min().unwrap() - PAD,
        xs.iter().max().unwrap() + PAD,
    );
    let (min_y, max_y) = (
        ys.iter().min().unwrap() - PAD,
        ys.iter().max().unwrap() + PAD,
    );

    let mut p = String::new();
    // Breite UND Hoehe setzen, im Verhaeltnis der viewBox. Nur `width` zu setzen laesst
    // Vorschau-Renderer (qlmanage, Quick Look) auf ein Quadrat zurueckfallen und schneidet
    // den Plan ab — die viewBox allein regelt das nicht.
    let (vw, vh) = (max_x - min_x, max_y - min_y);
    let out_w = 1100;
    let out_h = (out_w as f64 * vh as f64 / vw as f64).round() as i32;
    p.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{min_x} {min_y} {vw} {vh}" width="{out_w}" height="{out_h}">"##));
    p.push_str(&format!(
        r##"<rect x="{min_x}" y="{min_y}" width="{}" height="{}" fill="#FBFAF8"/>"##,
        max_x - min_x,
        max_y - min_y
    ));

    // Jeder Text, der schon auf dem Blatt steht. Die Moebelbeschriftungen weichen ihm aus.
    let mut gesetzt: Vec<Rect> = Vec::new();

    // Terrasse zuerst, damit die Aussenwand darueber liegt. Schraffiert, an den Laengsseiten
    // gestrichelt, nach aussen offen und mit "ca." beschriftet — vier Signaturen fuer dieselbe
    // Aussage: hier ist etwas, und niemand hat es gemessen. Eine Aussenflaeche, die aussieht wie
    // eine gemessene, ist eine Falle.
    //
    // Das Band selbst sperrt nichts: es ist Schraffur, und ein Name darueber bleibt lesbar. Sein
    // TEXT sperrt, wie jeder andere auch.
    if let Some(t) = &room.terrasse {
        let (x0, x1) = (t.x[0].max(min_x), t.x[1].min(max_x));
        let (y0, y1) = (t.y[0].max(min_y), t.y[1].min(max_y));
        if x1 > x0 && y1 > y0 {
            let (w, h) = (x1 - x0, y1 - y0);
            // Schraffur statt Flaeche, und je Strich an das Band geschnitten. Ein `<pattern>`
            // waere kuerzer und traegt eine Kennung: mehrere Plaene auf einer Seite — die
            // CLI-Seite zeigt alle — teilten sie sich dann.
            let mut s = x0 - h;
            while s < x1 {
                let lo = (x0 - s).max(0);
                let hi = (x1 - s).min(h);
                if hi > lo {
                    p.push_str(&format!(
                        r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#CFCAC0" stroke-width="2"/>"##,
                        s + lo, y0 + lo, s + hi, y0 + hi));
                }
                s += 30;
            }
            for kante in [y0, y1] {
                p.push_str(&format!(
                    r##"<line x1="{x0}" y1="{kante}" x2="{x1}" y2="{kante}" stroke="#9A968E" stroke-width="3" stroke-dasharray="18 12"/>"##
                ));
            }
            let (cx, cy) = (x0 + w / 2, y0 + h / 2);
            let name = format!("TERRASSE{}", if t.geschaetzt { " (ca.)" } else { "" });
            gesetzt.push(mittig(cx, cy, name.chars().count(), 20, 1, true));
            p.push_str(&format!(
                r##"<text x="{cx}" y="{cy}" transform="rotate(-90 {cx} {cy})" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">{name}</text>"##
            ));
        }
    }

    // Nur die Flaeche. Die Kontur wird nach den Moebeln gezeichnet, und der Absatz dort sagt,
    // warum das keine Kosmetik ist.
    let pts = poly
        .iter()
        .map(|q| format!("{},{}", q[0], q[1]))
        .collect::<Vec<_>>()
        .join(" ");
    p.push_str(&format!(r##"<polygon points="{pts}" fill="#F4F1EB"/>"##));

    if let Some(b) = &room.bad {
        let (w, h) = (b.x[1] - b.x[0], b.y[1] - b.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="#E2DDD2" stroke="#9A968E" stroke-width="3"/>"##,
            b.x[0], b.y[0]));
        let (cx, cy) = (b.x[0] + w / 2, b.y[0] + h / 2);
        gesetzt.push(mittig(cx, cy, 3, 20, 1, false));
        p.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">BAD</text>"##
        ));
    }

    for f in &room.fix_moebel {
        let (w, h) = (f.x[1] - f.x[0], f.y[1] - f.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="none" stroke="#6E655A" stroke-width="3" stroke-dasharray="12 8"/>"##,
            f.x[0], f.y[0]));
        // Mittig und mit dem Namen aus der Datei. Bis 2026-08-31 stand hier das Wort KUECHE
        // fuer jeden Einbau, 40 cm nach Sueden versetzt — bei einem Einbau an der Suedwand fiel
        // die Beschriftung damit AUS der Wohnung heraus.
        let name = f.id.to_uppercase();
        let (cx, cy) = (f.x[0] + w / 2, f.y[0] + h / 2);
        gesetzt.push(mittig(cx, cy, name.chars().count(), 20, 1, false));
        p.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">{}</text>"##,
            esc(&name)));
    }

    // Erst alle Grundflaechen, dann die Beschriftungen. Eine Beschriftung, die nach aussen
    // muss, braucht die Stuecke, die NACH ihr kommen — in einem Durchgang wuesste sie nur von
    // denen davor und legte sich auf das naechste Moebel.
    let mut stuecke: Vec<(&PlacedItem, Rect, String)> = Vec::with_capacity(layout.items.len());
    for it in &layout.items {
        let (w, d, _) = footprint(it, &model.catalogue)?;
        let label = short_label(&it.reference, it.kind.as_ref().map(|_| kind_of(it)));
        stuecke.push((
            it,
            Rect {
                x: it.x,
                y: it.y,
                w,
                d,
            },
            label,
        ));
    }
    let mut hindernisse: Vec<Rect> = stuecke.iter().map(|(_, r, _)| *r).collect();
    if let Some(b) = &room.bad {
        hindernisse.push(Rect {
            x: b.x[0],
            y: b.y[0],
            w: b.x[1] - b.x[0],
            d: b.y[1] - b.y[0],
        });
    }
    for f in &room.fix_moebel {
        hindernisse.push(Rect {
            x: f.x[0],
            y: f.y[0],
            w: f.x[1] - f.x[0],
            d: f.y[1] - f.y[0],
        });
    }
    let blatt = Rect {
        x: min_x,
        y: min_y,
        w: vw,
        d: vh,
    };
    let kern = Rect {
        x: min_x + PAD,
        y: min_y + PAD,
        w: vw - 2 * PAD,
        d: vh - 2 * PAD,
    };

    for (it, r, label) in &stuecke {
        let (w, d) = (r.w, r.d);
        // Rechteck und Beschriftung in EINER Gruppe, mit dem Ref als Griff.
        //
        // Gruppiert, weil beim Ziehen sonst die Beschriftung stehen bleibt und das Moebel ohne
        // sie wandert. `data-x`/`data-y` tragen die Modellkoordinaten in Zentimetern mit, damit
        // die Oberflaeche beim Loslassen nicht aus Bildschirmpixeln zurueckrechnen muss: die
        // viewBox laeuft ohnehin in Zentimetern, also ist die Umrechnung eine Matrixinversion
        // und keine eigene Skalenrechnung. Eine solche waere eine zweite Fassung der Geometrie.
        p.push_str(&format!(
            r##"<g data-ref="{}" data-x="{}" data-y="{}" data-w="{w}" data-d="{d}" data-rot="{}">"##,
            esc(&it.reference), r.x, r.y, it.rot));
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{d}" fill="#DAD2C4" stroke="#6E655A" stroke-width="3"/>"##,
            r.x, r.y));
        let s = setze(
            r,
            label.chars().count(),
            &hindernisse,
            &gesetzt,
            &blatt,
            &kern,
        );
        gesetzt.push(s.flaeche);
        if let Some([x1, y1, x2, y2]) = s.fuehrung {
            p.push_str(&format!(
                r##"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#6E655A" stroke-width="2"/>"##
            ));
        }
        // Nur `rotate(-90)`, nie `+90`: quer heisst von unten nach oben lesbar. Ein Text, der
        // mit der zweiten Drehrichtung gesetzt wird, steht auf dem Kopf.
        let dreh = if s.quer {
            format!(r##" transform="rotate(-90 {} {})""##, s.x, s.y)
        } else {
            String::new()
        };
        p.push_str(&format!(
            r##"<text x="{}" y="{}"{dreh} font-family="Helvetica,Arial" font-size="{}" fill="#16181A" text-anchor="{}" dominant-baseline="central">{}</text>"##,
            s.x, s.y, s.grad, s.anker, esc(label)));
        p.push_str("</g>");
    }

    // Die Huelle liegt OBEN, und das ist keine Kosmetik.
    //
    // Eine Kontur liegt mittig auf ihrer Linie, also deckt jedes Moebel an einer Wand die halbe
    // Wandstaerke zu. Bis 2026-08-31 wurde die Wand vor den Moebeln gezeichnet: im Bild
    // verschwand sie hinter jedem Stueck, das sie beruehrt, und der Schrank an der Nordwand sah
    // aus, als ragte er aus der Wohnung heraus. Die Zahlen waren dabei die ganze Zeit richtig.
    //
    // Die Oeffnungen gehoeren aus demselben Grund darueber, und zusaetzlich aus einem zweiten:
    // dass ein Schrank vor der Tuer steht, ist genau die Auskunft, fuer die jemand den Plan
    // ansieht. Glastuer voll, Fenster gestrichelt, Tuer grau — drei Signaturen, weil sie drei
    // verschiedene Dinge sind. Die TypeScript-Vorlage zeichnete Fenster wie Glastueren.
    p.push_str(&format!(
        r##"<polygon points="{pts}" fill="none" stroke="#16181A" stroke-width="5" stroke-linejoin="miter"/>"##
    ));
    for o in &room.oeffnungen {
        let Some((a, b)) = room.opening_span(o) else {
            continue;
        };
        let (stroke, dash) = match o.typ.as_deref() {
            Some("tuer") => ("#5B5F63", r##" stroke-dasharray="16 10""##),
            Some("fenster") => ("#B0764A", r##" stroke-dasharray="4 6""##),
            _ => ("#B0764A", ""),
        };
        p.push_str(&format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="9"{dash}/>"##,
            a[0], a[1], b[0], b[1]
        ));
    }

    p.push_str("</svg>");
    Ok(p)
}

/// Plan plus Verdikt in einer Datei — damit ein Bild nie ohne sein Urteil weitergereicht wird.
/// Woher die Positionen auf einem Blatt stammen.
///
/// Der Satz unter dem Titel behauptete bis 2026-09-26, nichts sei von Hand gesetzt. Fuer einen
/// Vorschlag stimmt das: jede Zahl kommt aus dem Modell. Fuer den Ist-Zustand stimmt es **nicht** —
/// dort ist jede Position eine Zeile in `interior_placement`, und die hat ein Mensch geschrieben.
/// Ein Blatt, das seine eigene Herkunft falsch nennt, ist genau die Sorte Aussage, gegen die der
/// Rest dieser Datei gebaut ist: es sieht aus wie ein gerechnetes Ergebnis und ist eine Eingabe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Herkunft {
    /// Ein Vorschlag: die Positionen stehen in `layouts/*.toml`, alle Zahlen aus dem Modell.
    Vorschlag,
    /// Der Zustand: die Positionen stehen in `interior_placement` und sind gesetzt worden.
    IstZustand,
}

pub fn page(model: &Model, layouts: &[Layout], herkunft: Herkunft) -> Result<String, ModelError> {
    // Der Titel kommt aus dem Modell. Hier stand der Name dieser einen Wohnung im Quelltext.
    let mut h = String::from(
        r##"<!doctype html><meta charset="utf-8"><title>Wohnung — Plaene</title>
<style>
:root{color-scheme:light dark}
body{margin:0;background:#FBFAF8;color:#16181A;font:16px/1.6 ui-sans-serif,-apple-system,Helvetica,Arial;padding:40px}
@media(prefers-color-scheme:dark){body{background:#131416;color:#E9E7E3}}
h1{font-size:1.8rem;margin:0 0 4px}
h2{font-size:1.15rem;margin:0}
.wrap{max-width:1180px;margin:0 auto}
.card{margin:38px 0;padding:24px;border:1px solid #C9C6C0;border-radius:14px;background:#fff}
@media(prefers-color-scheme:dark){.card{background:#1A1C1F;border-color:#3C4045}}
svg{width:100%;height:auto;display:block;margin:14px 0}
.pass{color:#2E7D32;font-weight:600}.fail{color:#B4402F;font-weight:600}
.warn{color:#9A6A00}
ul{margin:8px 0;padding-left:20px}
.meta{font:13px ui-monospace,Menlo,monospace;color:#5B5F63}
</style><div class="wrap"><h1>{TITEL} — Pläne</h1>
<p class="meta">Aus dem gemessenen Modell erzeugt. Jede Zahl stammt aus <code>room.toml</code>,
<code>rules.toml</code> und dem Inventar; {HERKUNFT}</p>"##)
        .replace("{TITEL}", &esc(&model.room.flat.name))
        .replace(
            "{HERKUNFT}",
            match herkunft {
                Herkunft::Vorschlag => "nichts auf dieser Seite ist von Hand gesetzt.",
                Herkunft::IstZustand => "die Positionen des Ist-Zustands hat ein Mensch gesetzt, \
                     das Modell liefert dazu nur den Raum und die Masse der Stuecke.",
            },
        );

    for l in layouts {
        let r = check_layout(model, l)?;
        h.push_str(&format!(
            r##"<div class="card"><h2>{}</h2><p class="{}">{}</p>"##,
            esc(&l.name),
            if r.pass { "pass" } else { "fail" },
            if r.pass {
                "bestanden — keine harte Regel verletzt"
            } else {
                "durchgefallen"
            }
        ));
        h.push_str(&svg(model, l)?);
        h.push_str(&format!(
            r##"<p class="meta">belegt {:.2} m² · frei {:.2} m² · {}</p>"##,
            r.metrics.occupied_area_m2,
            r.metrics.free_area_m2,
            r.metrics
                .corridors
                .iter()
                .map(|c| format!(
                    "{} → {}: {}",
                    c.from,
                    c.to,
                    c.width_cm.map_or("kein Weg".into(), |w| format!("{w} cm"))
                ))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
        if !r.hard.is_empty() {
            h.push_str("<ul>");
            for v in &r.hard {
                h.push_str(&format!(
                    r##"<li class="fail">{}: {}</li>"##,
                    v.rule,
                    esc(&v.message)
                ));
            }
            h.push_str("</ul>");
        }
        if !r.soft.is_empty() {
            h.push_str("<ul>");
            for v in &r.soft {
                h.push_str(&format!(
                    r##"<li class="warn">{}: {}</li>"##,
                    v.rule,
                    esc(&v.message)
                ));
            }
            h.push_str("</ul>");
        }
        if !r.uncertainties.is_empty() {
            h.push_str(&format!(
                r##"<p class="meta">gemessen? nein: {}</p>"##,
                r.uncertainties
                    .iter()
                    .map(|u| format!("{} ({})", esc(&u.label), u.fields.join(", ")))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        h.push_str("</div>");
    }
    h.push_str("</div>");
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die Faelle, die am 2026-08-31 im Bild standen, und was statt ihrer dort stehen soll.
    /// Jeder faellt aus einer der drei Regeln von `short_label` und nicht aus einer Tabelle.
    #[test]
    fn ein_kurzer_name_kommt_aus_dem_bezeichner() {
        for (bezeichner, erwartet) in [
            // Beiwerk faellt weg, eine abschliessende Zahl bleibt.
            ("pflanze_boden_1", "Pflanze 1"),
            ("bett_bestand", "Bett"),
            ("schreibtisch_gestell", "Schreibtisch"),
            ("couch_bestand", "Couch"),
            // Das Kopfwort steht hinten.
            ("kleiderschrank_bestand", "Schrank"),
            ("esstisch_midyou", "Tisch"),
            ("esstischstuehle", "Stuhl"),
            ("schreibtischplatte_bestand", "Platte"),
            ("saugroboter", "Sauger"),
            // Der Name schlaegt die geerbte Art: `kind_of_name` fuehrt dieses Stueck als Desk.
            ("buerostuhl", "Stuhl"),
            // Kein Kopfwort, keine deklarierte Art — dann das Wort selbst.
            ("kallax_regale", "Kallax"),
            ("luftreiniger", "Luftreiniger"),
        ] {
            assert_eq!(short_label(bezeichner, None), erwartet, "{bezeichner}");
        }
    }

    #[test]
    fn eine_deklarierte_art_traegt_einen_unbekannten_namen() {
        assert_eq!(short_label("pinntorp", Some(Kind::Table)), "Tisch");
        // `other` ist die Aussage "keine der bekannten Arten" und darf kein Wort stellen.
        assert_eq!(short_label("pinntorp", Some(Kind::Other)), "Pinntorp");
        // Und sie schlaegt den Namen nicht, wenn der eines hat.
        assert_eq!(short_label("buerostuhl", Some(Kind::Desk)), "Stuhl");
    }

    fn blatt() -> Rect {
        Rect {
            x: -100,
            y: -100,
            w: 800,
            d: 800,
        }
    }

    #[test]
    fn eine_beschriftung_die_hineinpasst_bleibt_drin() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 140,
            d: 80,
        };
        let s = setze(&r, "Bett".chars().count(), &[r], &[], &blatt(), &blatt());
        assert_eq!(
            s.grad, GRADE[0],
            "ein weites Rechteck traegt den grossen Grad"
        );
        assert!(!s.quer);
        assert!(s.fuehrung.is_none(), "innen braucht keine Fuehrungslinie");
        assert_eq!((s.x, s.y), (70, 40));
    }

    /// Ein Rechteck, das tiefer als breit ist, traegt seinen Text stehend statt ihn zu
    /// verlieren — und nimmt dafuer einen kleineren Grad in Kauf.
    #[test]
    fn ein_schmales_tiefes_rechteck_beschriftet_quer() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 45,
            d: 50,
        };
        let s = setze(&r, "Stuhl".chars().count(), &[r], &[], &blatt(), &blatt());
        assert!(s.quer);
        assert!(s.grad < GRADE[0]);
        assert!(breite(5, s.grad) + 2 * LUFT <= r.d);
        assert!(s.fuehrung.is_none());
    }

    /// Zu klein fuer jeden Grad: der Name geht nach draussen, haengt an einer Fuehrungslinie und
    /// legt sich auf kein Moebel.
    #[test]
    fn ein_zu_kleines_stueck_beschriftet_nach_aussen_ohne_etwas_zu_treffen() {
        let r = Rect {
            x: 200,
            y: 200,
            w: 35,
            d: 35,
        };
        let nachbar = Rect {
            x: 243,
            y: 180,
            w: 90,
            d: 90,
        };
        let s = setze(
            &r,
            "Sauger".chars().count(),
            &[r, nachbar],
            &[],
            &blatt(),
            &blatt(),
        );
        assert!(s.fuehrung.is_some(), "aussen braucht eine Fuehrungslinie");
        assert!(!s.flaeche.overlaps(&r));
        assert!(
            !s.flaeche.overlaps(&nachbar),
            "die Stelle nach Osten ist belegt, also darf sie nicht genommen werden"
        );
        assert!(frei(&s.flaeche, &[r, nachbar], &[], &blatt()));
    }

    /// Zwei gleiche Stuecke nebeneinander sind der Fall, an dem die alte Fassung zerbrach:
    /// beide trugen denselben Text an derselben Stelle.
    #[test]
    fn zwei_gleiche_nachbarn_bekommen_zwei_stellen() {
        let a = Rect {
            x: 0,
            y: 300,
            w: 40,
            d: 40,
        };
        let b = Rect {
            x: 45,
            y: 300,
            w: 40,
            d: 40,
        };
        let hindernisse = [a, b];
        let erste = setze(
            &a,
            "Pflanze 1".chars().count(),
            &hindernisse,
            &[],
            &blatt(),
            &blatt(),
        );
        let zweite = setze(
            &b,
            "Pflanze 2".chars().count(),
            &hindernisse,
            &[erste.flaeche],
            &blatt(),
            &blatt(),
        );
        assert!(!erste.flaeche.overlaps(&zweite.flaeche));
    }
}
