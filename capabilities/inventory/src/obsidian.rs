//! Der Weg eines Slots in den Vault, und der eine Block, den Axon darin besitzt.
//!
//! ## Warum diese Capability ueberhaupt in den Vault schreibt
//!
//! PRD Q58 (2026-08-30) hat Moebel auf Muster A gestellt: eine Zeile, kein Vault-Objekt, weil
//! Masse, Preis und Zustand durchgaengig schema-foermig sind. Fuer ein `piece` stimmt das und
//! bleibt so. Fuer einen `slot` stimmt es nicht, und die Messung sagt es deutlich: **13 der 18
//! Slots tragen eine `begruendung`, aber nur 3 der 29 Pieces** (gezaehlt 2026-09-06). Ein Slot ist
//! kein Ding — er ist eine offene Entscheidung mit Zielmassen, und §5.1b weist "Entscheidung"
//! nicht einer Tabelle zu, sondern dem Vault.
//!
//! §8.3 formuliert dieselbe Grenze von der anderen Seite: *"ein Dokument, das argumentiert, geht
//! in den Vault; eine Zahl, die abgefragt wird, geht nach Axon; eine Zahl, die ihr Argument
//! braucht, bleibt eine Datei in Axon"*. Der Slot-Text ist der erste Fall, die Masse sind der
//! zweite, und genau an dieser Naht schneidet dieses Modul.
//!
//! ## Warum `Atlas/`, und nicht `Resources/Sjel/`
//!
//! Q31 (2026-08-23) gibt Muster B genau ein Zuhause: `Resources/Sjel/`. Dieses Modul schreibt
//! nach `Atlas/Interior/`, und das ist **kein** Verstoss, weil es kein Muster B ist. Lars hat am
//! 2026-09-07 entschieden, dass eine Wohnungseinrichtung dauerhaft ist statt temporaer und
//! deshalb in den Atlas gehoert — und Q31s eigene Regel lautet *"prefer C"*: sobald eine
//! menschliche Notiz existiert, schreibt die Maschine eine markierte Region hinein und sonst
//! nichts. Diese Datei stellt genau das her.
//!
//! Der Bootstrap ist der einzige Sonderfall und er passiert **einmal pro Slot**: existiert keine
//! Notiz, wird eine angelegt, deren Prosa (`ziel`, `begruendung`, `hinweis`,
//! `entscheidung_offen`) **ausserhalb** der Marker landet und ab diesem Moment dem Menschen
//! gehoert. Danach fasst dieses Modul nur noch die Region an. Eine bestehende Notiz wird nie
//! ueberschrieben — §5.5 nennt das "Promotion, never demotion", und eine einmal geschriebene
//! menschliche Zeile ist genau die Sorte Satz, die kein Generator zurueckholen darf.
//!
//! ## Was in der Region steht, und was nicht
//!
//! In der Region stehen ausschliesslich Maschinenzahlen: Zielmasse, Platzbedarf, Preisspanne,
//! Prioritaet, Zustand, ersetzte Eintraege, Varianten, welche Masse geschaetzt sind. Nicht in der
//! Region steht ein einziges Wort, das ein Mensch geschrieben hat. Das ist dieselbe Trennung, die
//! `capabilities/finance/src/obsidian.rs` fuehrt — *"Neither writes the other's fields"* — und
//! der Grund, warum ein Konflikt hier ueberhaupt entscheidbar ist.

use std::collections::BTreeMap;
use std::path::PathBuf;

use markdown_root::{region, MarkdownRoot, RegionOutcome, RegionSpec, RootError};

use crate::store::{Item, Kind, State};

/// Der Besitzer der Marker. Fuer immer stabil: eine Aenderung macht jede bereits
/// geschriebene Region fremd, und der naechste Lauf haengt eine zweite daneben.
pub const REGION_OWNER: &str = "interior";

/// Wird erhoeht, wenn sich die Form des gerenderten Blocks aendert, damit ein spaeterer
/// Generator seine eigene alte Ausgabe von einer Form unterscheiden kann, die er nicht mehr
/// erzeugt.
pub const REGION_VERSION: u32 = 1;

/// Vault-relativ und nicht konfigurierbar, aus demselben Grund, den `trips::projection::DIR`
/// nennt: eine zweite Erklaerung, wohin Maschinenausgabe geht, ist der Weg, auf dem zwei Hosts
/// in zwei Ordner schreiben und es keiner merkt.
pub const DIR: &str = "Atlas/Interior";

/// Was ein Lauf getan hat. Jede Zeile ist eine Datei, damit der Bericht ohne Logdatei lesbar ist.
#[derive(Debug, Default)]
pub struct Report {
    /// Notizen, die es noch nicht gab und die einmalig angelegt wurden.
    pub seeded: Vec<String>,
    /// Notizen, deren Region neu geschrieben wurde.
    pub written: Vec<String>,
    /// Notizen, deren Region schon stimmte. Kein Schreibvorgang, also kein Commit im Vault.
    pub unchanged: Vec<String>,
    /// Notizen, deren Region ein Mensch angefasst hat. Nichts wurde geschrieben.
    pub conflicts: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Fehler {
    #[error("Vault-Wurzel: {0}")]
    Root(#[from] RootError),
    #[error("{pfad}: {quelle}")]
    Io {
        pfad: PathBuf,
        #[source]
        quelle: std::io::Error,
    },
    #[error("Region in {pfad}: {quelle}")]
    Region {
        pfad: PathBuf,
        #[source]
        quelle: region::RegionError,
    },
}

/// Der Dateiname eines Slots.
///
/// Aus dem `label` und nicht aus der `id`, weil ein Mensch die Notiz aufmacht und `[[Vorhang /
/// Verdunkelung Terrassentuer]]` lesbar ist, wo `[[vorhang_terrassentuer]]` es nicht ist. Der
/// Schraegstrich muss trotzdem weg — er ist auf jedem Dateisystem ein Verzeichnistrenner, und
/// ein Label darf keinen Ordner erfinden.
pub fn dateiname(item: &Item) -> String {
    let mut out = String::with_capacity(item.label.len());
    for ch in item.label.chars() {
        match ch {
            '/' | '\\' | ':' => out.push('-'),
            '#' | '^' | '[' | ']' | '|' => out.push(' '),
            c => out.push(c),
        }
    }
    let getrimmt = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if getrimmt.is_empty() {
        item.id.clone()
    } else {
        getrimmt
    }
}

fn zeile(out: &mut String, label: &str, wert: &str) {
    if !wert.is_empty() {
        out.push_str(&format!("> **{label}:** {wert}\n"));
    }
}

fn masse(item: &Item) -> String {
    let mut teile: Vec<String> = Vec::new();
    if let Some(b) = item.b {
        teile.push(format!("B {b}"));
    }
    if let Some(t) = item.t {
        teile.push(format!("T {t}"));
    }
    if let Some(h) = item.h {
        teile.push(format!("H {h}"));
    }
    if let Some(h) = item.h_min {
        teile.push(format!("H min {h}"));
    }
    if teile.is_empty() {
        String::new()
    } else {
        format!("{} cm", teile.join(" × "))
    }
}

fn euro(cent: i64) -> String {
    format!("{},{:02} €", cent / 100, (cent % 100).abs())
}

fn preis(item: &Item) -> String {
    match (item.preis_cent, item.kosten_min_cent, item.kosten_max_cent) {
        (Some(p), _, _) => euro(p),
        (None, Some(min), Some(max)) if min != max => format!("{} bis {}", euro(min), euro(max)),
        (None, Some(min), _) => format!("ab {}", euro(min)),
        (None, None, Some(max)) => format!("bis {}", euro(max)),
        _ => String::new(),
    }
}

/// Der Block, den Axon besitzt. Ausschliesslich Zahlen — kein Satz, den ein Mensch geschrieben
/// hat, wird hier reproduziert.
pub fn render_block(item: &Item, zustand: Option<State>) -> String {
    let mut out = String::new();
    out.push_str("> [!info] Von Axon abgeleitet — innerhalb dieses Blocks nichts eintragen\n");

    zeile(&mut out, "Zielmasse", &masse(item));
    zeile(&mut out, "Preis", &preis(item));
    if let Some(p) = item.prioritaet.as_deref().filter(|s| !s.trim().is_empty()) {
        zeile(&mut out, "Prioritaet", p);
    }
    if let Some(z) = zustand {
        zeile(&mut out, "Zustand", z.as_str());
    }
    if let Some(zone) = item.platzbedarf_zone {
        zeile(&mut out, "Platzbedarf Zone", &format!("{zone} cm"));
    }
    if let Some(block) = item.platzbedarf_block {
        zeile(&mut out, "Platzbedarf Block", &format!("{block} cm"));
    }
    if let Some(a) = item.anzahl.filter(|n| *n != 1) {
        zeile(&mut out, "Anzahl", &a.to_string());
    }
    if !item.ersetzt.is_empty() {
        zeile(&mut out, "Ersetzt", &item.ersetzt.join(", "));
    }
    if !item.varianten.is_empty() {
        zeile(&mut out, "Varianten", &item.varianten.join(", "));
    }
    if let Some(b) = item.basiert_auf.as_deref().filter(|s| !s.trim().is_empty()) {
        zeile(&mut out, "Basiert auf", b);
    }
    if let Some(l) = item.link.as_deref().filter(|s| !s.trim().is_empty()) {
        zeile(&mut out, "Link", l);
    }
    if let Some(q) = item.quelle.as_deref().filter(|s| !s.trim().is_empty()) {
        zeile(&mut out, "Quelle der Masse", q);
    }
    if let Some(g) = item.gemessen_am.as_deref().filter(|s| !s.trim().is_empty()) {
        zeile(&mut out, "Gemessen am", g);
    }

    // Welche Masse geschaetzt sind, steht hier und nicht im Fliesstext: es ist der Vorbehalt,
    // unter dem jede Zahl darueber gelesen werden muss, und ein Vorbehalt, den ein Mensch
    // versehentlich loeschen kann, ist keiner.
    if !item.unsicher.is_empty() {
        zeile(&mut out, "Geschaetzt", &item.unsicher.join(", "));
    }

    out.push_str(&format!(
        "\n> `{}` · aus der interior-Tabelle, nicht von Hand gepflegt.\n",
        item.id
    ));
    out
}

/// Die einmalige Saat: eine Notiz, deren Prosa dem Menschen gehoert.
///
/// Wird nur aufgerufen, wenn die Datei nicht existiert. Der Text aus `ziel`, `begruendung`,
/// `hinweis` und `entscheidung_offen` wird **ausserhalb** der Marker abgelegt und ist ab dann
/// nicht mehr Sache dieses Moduls.
fn saat(item: &Item, zustand: Option<State>) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("type: interior-slot\n");
    out.push_str(&format!("axon_interior_id: \"{}\"\n", item.id));
    out.push_str(&format!("summary: \"{}\"\n", item.label.replace('"', "'")));
    out.push_str("status: offen\n");
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", item.label));

    let mut abschnitt = |titel: &str, text: Option<&String>| {
        if let Some(t) = text.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            out.push_str(&format!("## {titel}\n\n{t}\n\n"));
        }
    };
    abschnitt("Ziel", item.ziel.as_ref());
    abschnitt("Begruendung", item.begruendung.as_ref());
    abschnitt("Hinweis", item.hinweis.as_ref());
    abschnitt("Offene Entscheidung", item.entscheidung_offen.as_ref());

    out.push_str(
        "> [!tip] Ab hier gehoert der Text dir\n\
         > Diese Notiz wurde einmalig aus der interior-Tabelle erzeugt. Alles ausserhalb des\n\
         > Axon-Blocks unten kannst du frei aendern — es wird nie ueberschrieben.\n\n",
    );

    // Die Region wird nicht hier gerendert: `region::apply` haengt sie an und traegt dabei den
    // Hash ein, der die Konflikterkennung traegt. Sie zweimal zu erzeugen hiesse, zwei Stellen
    // muessten sich ueber das Marker-Format einig sein.
    let _ = zustand;
    out
}

/// Jeden Slot in den Vault bringen: einmal saeen, danach nur noch die Region pflegen.
///
/// Pieces bleiben aussen vor. Sie sind Zeilen, und Q58 hat das entschieden — hier wird die
/// Entscheidung nur nicht heimlich aufgeweicht.
pub fn write_all(
    root: &MarkdownRoot,
    katalog: &BTreeMap<String, (Item, Option<State>)>,
) -> Result<Report, Fehler> {
    let mut report = Report::default();
    let spec = RegionSpec::new(REGION_OWNER, REGION_VERSION);

    let verzeichnis = root.path().join(DIR);
    std::fs::create_dir_all(&verzeichnis).map_err(|quelle| Fehler::Io {
        pfad: verzeichnis.clone(),
        quelle,
    })?;

    for (item, zustand) in katalog.values() {
        if item.kind != Kind::Slot {
            continue;
        }

        let relativ = format!("{DIR}/{}.md", dateiname(item));
        let pfad = root.locate(&relativ)?;
        let name = relativ.clone();

        let vorhanden = pfad.exists();
        if !vorhanden {
            std::fs::write(&pfad, saat(item, *zustand)).map_err(|quelle| Fehler::Io {
                pfad: pfad.clone(),
                quelle,
            })?;
            report.seeded.push(name.clone());
        }

        let original = std::fs::read_to_string(&pfad).map_err(|quelle| Fehler::Io {
            pfad: pfad.clone(),
            quelle,
        })?;
        let (aktualisiert, ausgang) =
            region::apply(&original, &spec, &render_block(item, *zustand)).map_err(|quelle| {
                Fehler::Region {
                    pfad: pfad.clone(),
                    quelle,
                }
            })?;

        match ausgang {
            RegionOutcome::Created | RegionOutcome::Updated => {
                std::fs::write(&pfad, aktualisiert).map_err(|quelle| Fehler::Io {
                    pfad: pfad.clone(),
                    quelle,
                })?;
                // Eine frisch gesaete Notiz zaehlt als `seeded`, nicht zusaetzlich als
                // `written`: der Bericht soll sagen, was neu ist, nicht wie oft die Datei
                // im selben Lauf angefasst wurde.
                if vorhanden {
                    report.written.push(name);
                }
            }
            RegionOutcome::Unchanged => {
                if vorhanden {
                    report.unchanged.push(name);
                }
            }
            RegionOutcome::Conflict { .. } => report.conflicts.push(name),
        }
    }

    Ok(report)
}

/// Die Vault-Wurzel dieser Maschine, falls eine erklaert ist.
///
/// `None` heisst: keine Bruecke, und das ist die richtige Antwort fuer einen Host ohne Vault —
/// kein Vault, keine Schreibvorgaenge, und die Zeilen bleiben trotzdem der Bestand.
///
/// **Zwei Konfigurationsnamen, in dieser Reihenfolge.** `inventory.json` ist der eigene Name
/// seit 2026-10-05; `interior.json` bleibt der Rueckfall, weil die Wurzel dort auf jeder
/// bestehenden Maschine schon steht und ein Umzug, der eine Konfigurationsdatei umbenennt,
/// nichts gewinnt. Dasselbe fuer die Umgebungsvariable.
pub fn vault_root() -> Option<PathBuf> {
    for name in [
        "SJEL_INVENTORY_OBSIDIAN_ROOT",
        "SJEL_INTERIOR_OBSIDIAN_ROOT",
    ] {
        if let Ok(p) = sjel_config::env_var(name) {
            if !p.trim().is_empty() {
                return Some(sjel_config::expand_tilde(&p));
            }
        }
    }
    for datei in ["inventory.json", "interior.json"] {
        let Some(pfad) = sjel_config::overlay_config(datei) else {
            continue;
        };
        let Some(text) = std::fs::read_to_string(pfad).ok() else {
            continue;
        };
        let Some(wert) = serde_json::from_str::<serde_json::Value>(&text).ok() else {
            continue;
        };
        if let Some(root) = wert
            .get("obsidian")
            .and_then(|o| o.get("root"))
            .and_then(|r| r.as_str())
        {
            return Some(sjel_config::expand_tilde(root));
        }
    }
    None
}

/// `write_all` gegen die erklaerte Wurzel, oder `None`, wenn keine erklaert ist.
pub fn writeback(
    katalog: &BTreeMap<String, (Item, Option<State>)>,
) -> Option<Result<Report, Fehler>> {
    let root_pfad = vault_root()?;
    Some(
        MarkdownRoot::declare(root_pfad)
            .map_err(Fehler::Root)
            .and_then(|root| write_all(&root, katalog)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: &str, label: &str) -> Item {
        Item {
            id: id.into(),
            kind: Kind::Slot,
            label: label.into(),
            ..Default::default()
        }
    }

    /// Ein Label mit Schraegstrich darf keinen Ordner erfinden. `Vorhang / Verdunkelung
    /// Terrassentuer` ist ein echter Eintrag, und ohne diese Ersetzung landet er in einem
    /// Unterverzeichnis `Vorhang `, das niemand erklaert hat.
    #[test]
    fn ein_schraegstrich_im_label_wird_kein_verzeichnis() {
        let it = slot("vorhang", "Vorhang / Verdunkelung Terrassentuer");
        let name = dateiname(&it);
        assert!(!name.contains('/'), "got: {name}");
        assert_eq!(name, "Vorhang - Verdunkelung Terrassentuer");
    }

    /// Faellt das Label ganz weg, ist die id der Name. Eine Datei `.md` ohne Stamm waere in
    /// Obsidian unsichtbar, was schlimmer ist als ein haesslicher Name.
    #[test]
    fn ein_leeres_label_faellt_auf_die_id_zurueck() {
        let it = slot("kleiderschrank", "   ");
        assert_eq!(dateiname(&it), "kleiderschrank");
    }

    /// Der Kern der Trennung: kein menschlicher Satz wird in die Region kopiert. Steht die
    /// Begruendung erst einmal in beiden Haelften, entscheidet der naechste Lauf zwischen zwei
    /// Fassungen — und genau das soll die Region nie muessen.
    #[test]
    fn die_region_reproduziert_keinen_menschlichen_satz() {
        let mut it = slot("stuhl", "Zweiter Stuhl");
        it.begruendung = Some("Ein zweiter Stuhl ist eine Garderobe mit Beinen.".into());
        it.ziel = Some("Klappbar, unter 40 cm tief.".into());
        it.hinweis = Some("Erst nach der Tisch-Entscheidung.".into());
        it.entscheidung_offen = Some("Klappbar oder gar nicht?".into());
        it.b = Some(45);

        let block = render_block(&it, Some(State::Wanted));
        for satz in [
            "Garderobe mit Beinen",
            "Klappbar, unter 40",
            "Erst nach der Tisch",
            "Klappbar oder gar nicht",
        ] {
            assert!(!block.contains(satz), "Region traegt Prosa: {satz}");
        }
        assert!(block.contains("B 45 cm"), "got: {block}");
        assert!(block.contains("wanted"), "got: {block}");
    }

    /// Die Saat traegt die Prosa, und zwar ausserhalb jedes Markers. Sie ist der einzige
    /// Moment, in dem dieses Modul einen menschlichen Satz schreibt.
    #[test]
    fn die_saat_traegt_die_prosa_und_keinen_marker() {
        let mut it = slot("stuhl", "Zweiter Stuhl");
        it.begruendung = Some("Ein zweiter Stuhl ist eine Garderobe mit Beinen.".into());
        let text = saat(&it, Some(State::Wanted));
        assert!(text.contains("Garderobe mit Beinen"), "got: {text}");
        assert!(
            !text.contains("axon:begin"),
            "die Saat rendert keine Region"
        );
        assert!(text.contains("axon_interior_id: \"stuhl\""), "got: {text}");
    }

    /// Ein Piece ist eine Zeile. Q58 hat das entschieden, und ein Lauf, der still doch eine
    /// Notiz anlegt, weicht die Entscheidung auf, ohne sie zu widerrufen.
    #[test]
    fn ein_piece_bekommt_keine_notiz() {
        let dir = std::env::temp_dir().join(format!("axon-interior-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();

        let mut katalog: BTreeMap<String, (Item, Option<State>)> = BTreeMap::new();
        let mut piece = slot("tisch_bestand", "Tisch vorhanden");
        piece.kind = Kind::Piece;
        katalog.insert("tisch_bestand".into(), (piece, Some(State::Owned)));
        katalog.insert(
            "vorhang".into(),
            (
                slot("vorhang", "Vorhang Terrassentuer"),
                Some(State::Wanted),
            ),
        );

        let report = write_all(&root, &katalog).unwrap();
        assert_eq!(
            report.seeded,
            vec!["Atlas/Interior/Vorhang Terrassentuer.md"]
        );
        assert!(!dir.join(DIR).join("Tisch vorhanden.md").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Der zweite Lauf schreibt nichts. Ein No-Op-Schreibvorgang waere ein Commit im Vault,
    /// der nichts sagt, und der Vault ist ein Git-Repository.
    #[test]
    fn ein_zweiter_lauf_ohne_aenderung_schreibt_nicht() {
        let dir = std::env::temp_dir().join(format!("axon-interior-2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();

        let mut katalog: BTreeMap<String, (Item, Option<State>)> = BTreeMap::new();
        katalog.insert(
            "vorhang".into(),
            (
                slot("vorhang", "Vorhang Terrassentuer"),
                Some(State::Wanted),
            ),
        );

        let erst = write_all(&root, &katalog).unwrap();
        assert_eq!(erst.seeded.len(), 1);
        let zweit = write_all(&root, &katalog).unwrap();
        assert!(zweit.seeded.is_empty(), "got: {:?}", zweit.seeded);
        assert!(zweit.written.is_empty(), "got: {:?}", zweit.written);
        assert_eq!(zweit.unchanged.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Eine vom Menschen geaenderte Region wird nicht ueberschrieben, und die Prosa daneben
    /// ueberlebt jeden Lauf. Das ist die Zusage, unter der diese Notizen im Atlas stehen
    /// duerfen statt in `Resources/Sjel/`.
    #[test]
    fn menschliche_prosa_und_eine_beruehrte_region_ueberleben() {
        let dir = std::env::temp_dir().join(format!("axon-interior-3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();

        let mut katalog: BTreeMap<String, (Item, Option<State>)> = BTreeMap::new();
        katalog.insert(
            "vorhang".into(),
            (
                slot("vorhang", "Vorhang Terrassentuer"),
                Some(State::Wanted),
            ),
        );
        write_all(&root, &katalog).unwrap();

        let pfad = dir.join(DIR).join("Vorhang Terrassentuer.md");
        let mit_prosa = std::fs::read_to_string(&pfad)
            .unwrap()
            .replace(
                "# Vorhang Terrassentuer",
                "# Vorhang Terrassentuer\n\nMein eigener Satz.",
            )
            .replace("wanted", "von Hand veraendert");
        std::fs::write(&pfad, &mit_prosa).unwrap();

        let report = write_all(&root, &katalog).unwrap();
        assert_eq!(
            report.conflicts,
            vec!["Atlas/Interior/Vorhang Terrassentuer.md"]
        );
        let danach = std::fs::read_to_string(&pfad).unwrap();
        assert_eq!(danach, mit_prosa, "nichts wurde geschrieben");
        assert!(danach.contains("Mein eigener Satz."));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
