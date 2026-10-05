//! `interior wunsch <url>` — ein geteilter Link wird eine Wunschzeile.
//!
//! Das ist der Intake, den der ISA bis zum 2026-10-05 als *"scan an object, share a link, one
//! wishlist"* unter "Not yet specified" fuehrte. Gebaut ist davon genau der Link: die Zeile
//! entsteht in derselben Tabelle wie jedes Moebel (PRD Q58), mit `link`, `preis_cent` und
//! einem Zustand `wanted` — also dort, wo die Wunschliste schon gegen den Monatssaldo
//! rechnet (B29).
//!
//! **Dieses Modul liest einen Link; die Zeile schreibt `main.rs`.** Die Trennung ist die
//! Schnittlinie, an der sich pruefen laesst: alles hier ist rein bis auf [`holen`], und genau
//! der Rest laesst sich ohne Netz pruefen.
//!
//! ## Was gelesen wird und was nicht
//!
//! Gelesen werden **nur Angaben, die die Seite selbst deklariert**: `og:title` vor `<title>`,
//! und ein Preis ausschliesslich aus einem Meta-Tag (`og:price:amount`,
//! `product:price:amount`, `itemprop="price"`). Nicht gelesen wird ein Preis, der irgendwo im
//! Fliesstext zwischen zwei Zahlen steht.
//!
//! Der Grund ist die Richtung, in die ein Fehler hier faellt: `preis_cent` summiert sich in
//! `budget::kaufreihenfolge` und in die Wunschsumme, die gegen `finance` steht. Ein zu hoch
//! geratener Preis ist damit kein kosmetischer Fehler, sondern eine Zahl, die eine
//! Kaufentscheidung verschiebt. Eine Zeile **ohne** Preis wird von `GET /api/wishlist` als
//! `posten_ohne_preis` gezaehlt und gesagt; eine Zeile mit einem erfundenen Preis nicht.
//!
//! Eine Waehrung, die nicht Euro ist, wird abgelehnt und nicht umgerechnet: der Kurs steht
//! nirgends in dieser Datenbank, und eine Umrechnung waere eine Zahl ohne Herkunft.

use std::time::Duration;

/// Warum aus einem Link keine Zeile wurde.
///
/// Zwei Faelle, und der Unterschied ist keine Kosmetik: eine **abgelehnte** URL ist eine
/// Entscheidung (kein http(s), oder eine Adresse in diesem Netz) und damit das Ende des
/// Aufrufs. Eine **nicht erreichte** Seite ist eine Umstandsbedingung — ein Laden, der nicht
/// antwortet, darf die Zeile nicht verhindern, denn der Link ist der Punkt und der Titel die
/// Zugabe. Beides in einen `String` zu werfen hiesse, einem `file://`-Link dieselbe Nachsicht
/// zu geben wie einem Timeout, und genau das hat beim ersten Lauf eine Zeile namens `passwd`
/// angelegt.
#[derive(Debug)]
pub enum Abruf {
    Abgelehnt(String),
    NichtErreicht(String),
}

impl std::fmt::Display for Abruf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Abruf::Abgelehnt(e) | Abruf::NichtErreicht(e) => f.write_str(e),
        }
    }
}

/// Wie lange auf eine Produktseite gewartet wird. Ein Laden, der nicht antwortet, ist keine
/// Zeile wert, die man spaeter nachtraegt.
const ZEITGRENZE: Duration = Duration::from_secs(20);

/// Ein Browser-User-Agent, und der Grund steht hier statt im Client.
///
/// Laeden liefern einem selbst identifizierenden Client **gar nichts**. Gemessen am 2026-10-05
/// an einer Produktseite: `Sjel-interior-wunsch/0.0.1` bekam ueber HTTP/2 `stream 1 was not
/// closed cleanly: INTERNAL_ERROR (err 2)` und ueber HTTP/1.1 einen Timeout; derselbe Abruf
/// mit diesem Agenten 200 und 1,1 MB. Der Agent ist damit keine Bequemlichkeit, sondern die
/// Bedingung dafuer, dass dieser Befehl ueberhaupt liest.
///
/// Dieselbe Notwendigkeit wie in `capabilities/scouting/src/adapters/meetup.rs` und
/// `capabilities/transit/src/hafas.rs`, und dieselbe Regel: benannt und nicht getarnt.
/// `sjel_http::builder` existiert genau fuer diesen Fall — `client` setzt den Agenten, und
/// `builder` ist der Weg, ihn zu ersetzen.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

/// Was aus einer Seite gelesen wurde. Jedes Feld ist optional, weil jedes fehlen darf: eine
/// Zeile ohne Preis wird gezaehlt, eine ohne Titel wird abgelehnt.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Gelesen {
    pub titel: Option<String>,
    pub preis_cent: Option<i64>,
    /// Das Meta-Tag, aus dem die Zahl kam — damit ein falscher Preis eine Spur hat, statt
    /// einfach dazustehen.
    pub preis_aus: Option<String>,
}

/// Holt eine Produktseite und liest, was sie ueber sich selbst sagt.
///
/// Die beiden Wachen stehen **vor** der Anfrage und kommen aus `sjel_http::guard`: ohne sie
/// koennte ein geteilter Link auf `file://` oder auf eine Adresse in diesem Netz zeigen.
/// Dieselbe Reihenfolge wie `comms::media::fetch` — eine Anfrage, eine Stelle, an der sie
/// geprueft wird.
///
/// Der Fehler ist ein `String` und kein `Box<dyn Error>`, weil dieser Aufruf auf einem
/// Blockier-Thread laeuft (`tokio::task::spawn_blocking`) und dessen Ergebnis `Send` sein
/// muss. Ein `String` traegt dieselbe Auskunft und diese Bedingung.
pub fn holen(url: &str) -> Result<Gelesen, Abruf> {
    sjel_http::guard::check_scheme(url).map_err(|e| Abruf::Abgelehnt(e.to_string()))?;
    // Keine Allowlist: die Ausnahme in `check_destination` gibt es fuer `tools/demo-up`, das
    // sich selbst ueber Loopback bedient. Ein Mensch, der einen Link einsetzt, hat sie nicht.
    sjel_http::guard::check_destination(url, Vec::new)
        .map_err(|e| Abruf::Abgelehnt(e.to_string()))?;

    // `builder` und nicht `client`: der Agent oben ist der Grund. Nicht gecacht, weil eine
    // zusaetzliche Option fuer `client` unsichtbar waere — bei einem Aufruf je Link ohne
    // Belang.
    let client = sjel_http::builder(sjel_http::Purpose::new("interior-wunsch"), ZEITGRENZE)
        .user_agent(BROWSER_UA)
        .build()
        .map_err(|e| Abruf::NichtErreicht(e.to_string()))?;
    let antwort = client
        .get(url)
        .send()
        // `without_url` wie in `trips::interior_client`: die URL steht schon im Aufruf des
        // Lesers, und reqwest druckt sie sonst ganz in die Meldung.
        .map_err(|e| Abruf::NichtErreicht(e.without_url().to_string()))?;
    let status = antwort.status();
    if !status.is_success() {
        return Err(Abruf::NichtErreicht(format!("{url}: {status}")));
    }
    let seite = antwort
        .text()
        .map_err(|e| Abruf::NichtErreicht(e.to_string()))?;
    Ok(lesen(&seite))
}

/// Was in einer Seite steht, ohne Netz. Getrennt von [`holen`], damit genau das geprueft
/// werden kann, was hier die Fehler machen kann.
pub fn lesen(seite: &str) -> Gelesen {
    Gelesen {
        titel: titel(seite),
        preis_cent: preis(seite).map(|(c, _)| c),
        preis_aus: preis(seite).map(|(_, aus)| aus),
    }
}

/// `og:title` vor `<title>`.
///
/// `og:title` gewinnt, weil ein Laden in `<title>` seine Marke und den Zusatz haengt
/// (*"Hemd weiss | Beispielshop"*) und in `og:title` das Produkt. Fehlt beides, hat die Zeile
/// keinen Titel und der Aufrufer faellt auf die Kennung zurueck — eine Zeile ohne Namen
/// anzulegen waere eine, die niemand wiederfindet.
pub fn titel(seite: &str) -> Option<String> {
    meta(seite, "og:title")
        .or_else(|| element(seite, "title"))
        .map(|t| putzen(&t))
        .filter(|t| !t.is_empty())
}

/// Ein deklarierter Preis und das Tag, das ihn deklariert hat.
///
/// Reihenfolge nach Aussagekraft: `og:price:amount` und `product:price:amount` sind
/// Produktdaten fuer Vorschaukarten, `itemprop="price"` ist schema.org und damit die
/// ausdrueckliche Angabe der Seite. Was nicht in einem dieser drei steht, ist kein Preis,
/// sondern eine Zahl im Text.
pub fn preis(seite: &str) -> Option<(i64, String)> {
    for tag in ["og:price:amount", "product:price:amount", "price"] {
        let Some(roh) = meta(seite, tag) else {
            continue;
        };
        // Eine Waehrung, die nicht Euro ist, wird gemeldet und nicht umgerechnet.
        if let Some(waehrung) = waehrung(seite) {
            if !waehrung.eq_ignore_ascii_case("EUR") {
                return None;
            }
        }
        if let Some(cent) = betrag_cent(&roh) {
            return Some((cent, tag.to_string()));
        }
    }
    None
}

/// Die deklarierte Waehrung, falls die Seite eine nennt.
fn waehrung(seite: &str) -> Option<String> {
    meta(seite, "og:price:currency").or_else(|| meta(seite, "product:price:currency"))
}

/// `79.90`, `79,90`, `79.90 EUR`, `EUR 79.90` -> `7990`.
///
/// Streng und absichtlich: nach dem Abziehen eines Waehrungszeichens muss genau eine Zahl
/// uebrig bleiben. `ab 79.90` oder `79.90 - 129.00` sind damit **kein** Preis, sondern eine
/// Auskunft, die dieser Leser nicht sicher deuten kann — und `None` heisst hier "nicht
/// gelesen", nicht "null".
///
/// Der erste Versuch war zu grosszuegig und hat es selbst widerlegt: ein
/// `trim_start_matches(is_alphabetic)` schnitt das `ab` von `ab 79` ab und machte daraus
/// stillschweigend 79,00 € — in einer Spalte, die sich in die Wunschsumme summiert. Der
/// Modulkopf behauptete das Gegenteil, und kein Test widersprach ihm, weil keiner `ab 79`
/// nannte.
pub fn betrag_cent(roh: &str) -> Option<i64> {
    let s = ohne_waehrung(roh);
    // Tausendertrennzeichen weg, wo beide Zeichen vorkommen: `1.234,56`.
    let s = if s.contains('.') && s.contains(',') {
        s.replace('.', "")
    } else {
        s.to_string()
    };
    let s = s.replace(',', ".");
    let (ganz, bruch) = match s.split_once('.') {
        Some((g, b)) => (g, b),
        None => (s.as_str(), ""),
    };
    if ganz.is_empty() || !ganz.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !bruch.bytes().all(|b| b.is_ascii_digit()) || bruch.len() > 2 {
        return None;
    }
    let ganz: i64 = ganz.parse().ok()?;
    // `79.9` ist 79,90 und nicht 79,09 — eine Nachkommastelle ist die zweite Stelle, die
    // fehlt.
    let bruch: i64 = match bruch.len() {
        0 => 0,
        1 => bruch.parse::<i64>().ok()? * 10,
        _ => bruch.parse().ok()?,
    };
    Some(ganz * 100 + bruch)
}

/// `EUR`, `€` oder `$` an einem der beiden Enden — und sonst nichts. Alles andere ist Text und
/// bleibt stehen, damit die Ziffernpruefung ihn sieht.
fn ohne_waehrung(s: &str) -> &str {
    let t = s.trim();
    let ohne_zeichen = t
        .strip_prefix(['€', '$'])
        .or_else(|| t.strip_suffix(['€', '$']))
        .map(str::trim)
        .unwrap_or(t);
    // `to_ascii_uppercase` laesst die Laenge stehen, also stimmen die Indizes danach.
    let gross = ohne_zeichen.to_ascii_uppercase();
    if gross.starts_with("EUR") {
        return ohne_zeichen[3..].trim();
    }
    if gross.ends_with("EUR") {
        return ohne_zeichen[..ohne_zeichen.len() - 3].trim();
    }
    ohne_zeichen
}

/// Eine Kennung aus einem Titel: `"Oxford Hemd, weiss"` -> `"oxford-hemd-weiss"`.
///
/// ASCII und klein, weil die Kennung in einer URL steht (`/api/items/{id}`) und in einer
/// Obsidian-Datei landet (`obsidian::dateiname`). Umlaute werden umgeschrieben statt
/// entfernt: `Groesse` waere sonst ein anderes Wort als das, was im Laden stand.
pub fn kennung(titel: &str) -> String {
    let mut out = String::new();
    for c in titel.chars() {
        let c = match c {
            'ä' | 'Ä' => 'a',
            'ö' | 'Ö' => 'o',
            'ü' | 'Ü' => 'u',
            'ß' => 's',
            other => other,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Die Kennung, die noch frei ist: `hemd`, sonst `hemd-2`, `hemd-3` …
///
/// Ein zweites Hemd ist ein zweiter Eintrag und nicht ein ueberschriebener — dieselbe Regel
/// wie bei `interior_item_state`. Eine leere Vorlage wird `eintrag`, damit ein Titel aus
/// lauter Sonderzeichen keine Zeile ohne Kennung erzeugt.
pub fn freie_kennung(vorschlag: &str, vergeben: impl Fn(&str) -> bool) -> String {
    let basis = if vorschlag.is_empty() {
        "eintrag".to_string()
    } else {
        vorschlag.to_string()
    };
    if !vergeben(&basis) {
        return basis;
    }
    // ponytail: lineare Suche ab 2. Ein Mensch legt keine 10.000 Hemden an; waere es ein
    // Import, gehoerte hier die Zahl aus `COUNT(*)`.
    for n in 2..10_000 {
        let kandidat = format!("{basis}-{n}");
        if !vergeben(&kandidat) {
            return kandidat;
        }
    }
    basis
}

// --- HTML, so wenig wie moeglich ---------------------------------------------------------
//
// Kein Parser und keine Abhaengigkeit. Gebraucht werden genau zwei Dinge: der Inhalt eines
// `<meta>`-Tags und der eines `<title>`. Beides steht im Kopf einer Seite, beides ist mit
// einem Durchlauf gefunden, und `libs/extraction` liefert Text ohne diese Felder.

/// Der `content` eines `<meta>`, dessen `property`, `name` oder `itemprop` so heisst.
fn meta(seite: &str, gesucht: &str) -> Option<String> {
    let mut rest = seite;
    while let Some(start) = finde(rest, "<meta") {
        rest = &rest[start..];
        let ende = rest.find('>')?;
        let tag = &rest[..ende];
        rest = &rest[ende + 1..];
        let attribute = attribute(tag);
        let name = attribute
            .iter()
            .find(|(k, _)| matches!(k.as_str(), "property" | "name" | "itemprop"))
            .map(|(_, v)| v.as_str());
        if name.is_some_and(|n| n.eq_ignore_ascii_case(gesucht)) {
            return attribute
                .into_iter()
                .find(|(k, _)| k == "content")
                .map(|(_, v)| v);
        }
    }
    None
}

/// Der Inhalt des ersten `<title>`.
fn element(seite: &str, name: &str) -> Option<String> {
    let auf = format!("<{name}");
    let zu = format!("</{name}");
    let start = finde(seite, &auf)?;
    let rest = &seite[start..];
    let inhalt_start = rest.find('>')? + 1;
    let inhalt_ende = finde(&rest[inhalt_start..], &zu)?;
    Some(rest[inhalt_start..inhalt_start + inhalt_ende].to_string())
}

/// `find`, aber ohne Ruecksicht auf Gross- und Kleinschreibung. `str::to_lowercase` wuerde
/// den Index verschieben, sobald ein Zeichen dabei laenger wird.
fn finde(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Die Attribute eines Tags, ohne die spitzen Klammern. Reihenfolge bleibt erhalten, damit
/// `property` vor `content` gefunden wird und umgekehrt.
fn attribute(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Name
        while i < bytes.len() && !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'-' {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name = tag[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        let wert = if quote == b'"' || quote == b'\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let w = tag[start..i].to_string();
            i += 1;
            w
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            tag[start..i].to_string()
        };
        out.push((name, wert));
    }
    out
}

/// Entities aufloesen und Weissraum zusammenziehen. Ein Titel aus einer Seite traegt
/// Zeilenumbrueche und Einrueckung, und beides hat in einem Label nichts zu suchen.
fn putzen(roh: &str) -> String {
    let mut out = String::new();
    let mut rest = roh;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let Some(ende) = rest[i..].find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[i + 1..];
            continue;
        };
        let entity = &rest[i + 1..i + ende];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "nbsp" => out.push(' '),
            "ndash" => out.push('-'),
            "mdash" => out.push('-'),
            "szlig" => out.push('s'),
            other => match zahl(other) {
                Some(c) => out.push(c),
                // Unbekannt heisst stehenlassen, nicht verschlucken.
                None => {
                    out.push('&');
                    out.push_str(entity);
                    out.push(';');
                }
            },
        }
        rest = &rest[i + ende + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `#39` und `#x27`.
fn zahl(entity: &str) -> Option<char> {
    let rest = entity.strip_prefix('#')?;
    let code = match rest.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => rest.parse().ok()?,
    };
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eine erfundene Produktseite. Keine echten Laeden und keine echten Preise — dieselbe
    /// Regel wie in `tests/containment.rs`, das eine Wohnung nicht in `src/` duldet.
    const SEITE: &str = r#"<!doctype html>
<html><head>
  <meta charset="utf-8">
  <title>Beispielhemd weiss | Beispielshop</title>
  <meta property="og:title" content="Beispielhemd, weiss &amp; buegelfrei">
  <meta property="og:price:amount" content="79.90">
  <meta property="og:price:currency" content="EUR">
</head><body><p>Ab 79.90 EUR</p></body></html>"#;

    #[test]
    fn og_titel_schlaegt_title() {
        assert_eq!(
            titel(SEITE).as_deref(),
            Some("Beispielhemd, weiss & buegelfrei")
        );
    }

    #[test]
    fn ohne_og_titel_faellt_es_auf_title_zurueck() {
        let seite = "<head><title> Nur der Titel </title></head>";
        assert_eq!(titel(seite).as_deref(), Some("Nur der Titel"));
    }

    /// Der Kern der Vorsicht: eine Zahl im Fliesstext ist kein Preis. Sonst waere
    /// `Ab 79.90 EUR` ueber `itemprop` erreichbar, obwohl die Seite gar keins deklariert.
    #[test]
    fn eine_zahl_im_text_ist_kein_preis() {
        let seite = "<html><body>Ab 79.90 EUR, versandkostenfrei</body></html>";
        assert_eq!(preis(seite), None);
        assert_eq!(lesen(seite).preis_cent, None);
    }

    #[test]
    fn ein_deklarierter_preis_wird_gelesen_und_benannt() {
        let (cent, aus) = preis(SEITE).expect("Preis aus og:price:amount");
        assert_eq!(cent, 7990);
        assert_eq!(aus, "og:price:amount");
    }

    #[test]
    fn itemprop_ist_der_dritte_weg() {
        let seite = r#"<meta itemprop="price" content="1.234,56">"#;
        assert_eq!(preis(seite), Some((123456, "price".to_string())));
    }

    /// Eine fremde Waehrung wird abgelehnt und nicht umgerechnet. Der Kurs steht nirgends in
    /// dieser Datenbank; eine Zahl daraus waere eine ohne Herkunft.
    #[test]
    fn eine_fremde_waehrung_wird_abgelehnt() {
        let seite = r#"<meta property="og:price:amount" content="79.90">
                       <meta property="og:price:currency" content="USD">"#;
        assert_eq!(preis(seite), None);
    }

    /// Attribute stehen in beiden Reihenfolgen auf der Seite, und `content` vor `property`
    /// ist die, an der eine Suche nach dem ersten `property` scheitert.
    #[test]
    fn die_attributreihenfolge_ist_egal() {
        let a = r#"<meta content="12,50" property="og:price:amount">"#;
        let b = r#"<meta property="og:price:amount" content="12,50">"#;
        assert_eq!(preis(a), Some((1250, "og:price:amount".to_string())));
        assert_eq!(preis(b), Some((1250, "og:price:amount".to_string())));
    }

    #[test]
    fn betraege_die_dieser_leser_nicht_deuten_kann_sind_kein_preis() {
        // Eine Spanne ist kein Preis.
        assert_eq!(betrag_cent("79.90 - 129.00"), None);
        // Drei Stellen hinter dem Komma sind keine Cent.
        assert_eq!(betrag_cent("79.901"), None);
        // Nichts Zahlartiges.
        assert_eq!(betrag_cent("kostenlos"), None);
        // **Der Fall, an dem die erste Fassung gescheitert ist.** `ab` ist Text und keine
        // Waehrung; wer ihn abschneidet, macht aus einer Angabe einen Preis.
        assert_eq!(betrag_cent("ab 79"), None);
        assert_eq!(betrag_cent("ab 79,90"), None);
        assert_eq!(betrag_cent("statt 99"), None);
        // Eine Tausenderzahl ohne Nachkommastellen wird nicht gedeutet.
        assert_eq!(betrag_cent("1.234"), None);
        // Und die Faelle, die es treffen muss:
        assert_eq!(betrag_cent("79.90"), Some(7990));
        assert_eq!(betrag_cent("79,90"), Some(7990));
        assert_eq!(betrag_cent("79,9"), Some(7990));
        assert_eq!(betrag_cent("79"), Some(7900));
        assert_eq!(betrag_cent("1.234,56"), Some(123456));
        assert_eq!(betrag_cent("79.90 EUR"), Some(7990));
        assert_eq!(betrag_cent("EUR 79.90"), Some(7990));
        assert_eq!(betrag_cent("79,90 eur"), Some(7990));
        assert_eq!(betrag_cent("€ 79,90"), Some(7990));
    }

    #[test]
    fn eine_kennung_ist_ascii_klein_und_steht_in_der_url() {
        assert_eq!(kennung("Oxford Hemd, weiss"), "oxford-hemd-weiss");
        assert_eq!(kennung("Groesse 42 / blau"), "groesse-42-blau");
        assert_eq!(kennung("  --  "), "");
    }

    /// Ein zweites Hemd ist ein zweiter Eintrag. Dieselbe Regel wie bei
    /// `interior_item_state`: ein Kauf ueberschreibt keine Zeile.
    #[test]
    fn eine_belegte_kennung_bekommt_eine_zweite() {
        let belegt = |k: &str| k == "hemd" || k == "hemd-2";
        assert_eq!(freie_kennung("hemd", belegt), "hemd-3");
        assert_eq!(freie_kennung("mantel", belegt), "mantel");
        // Ein Titel aus lauter Sonderzeichen erzeugt keine Zeile ohne Kennung.
        assert_eq!(freie_kennung("", belegt), "eintrag");
    }

    #[test]
    fn entities_und_weissraum_werden_geputzt() {
        assert_eq!(putzen("  Hemd\n   weiss  "), "Hemd weiss");
        assert_eq!(
            putzen("A &amp; B &#39;x&#39; &unbekannt;"),
            "A & B 'x' &unbekannt;"
        );
    }
}
