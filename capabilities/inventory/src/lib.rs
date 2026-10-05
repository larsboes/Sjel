//! inventory — was ich besitze.
//!
//! PRD Q58 (2026-08-30): **eine Item-Tabelle, zwei Konsumenten.** Ein Zelt und ein
//! Kleiderschrank sind dieselbe Zeilenform — ein Ding mit Massen, einem Preis, einer Herkunft
//! und einem Zustand. Was sich unterscheidet, ist die **Platzierung**, und die ist eine zweite
//! Tabelle, keine zweite Kopie.
//!
//! ## Warum das eine eigene Capability ist
//!
//! Bis 2026-10-05 lagen diese Zeilen in `capabilities/interior`, und `interior` ist absichtlich
//! on-demand: es liefert einen Grundriss und private Fotos einer Wohnung aus. Nur hing an ihm
//! auch das Inventar — `capabilities/trips` liest `GET /api/inventory` ueber HTTP und meldet
//! `interior_reachable: false`, wenn nichts lauscht, also verlor nach jedem Neustart jede
//! Packliste ihre Gewichte, bis jemand den Grundriss oeffnete. Zwei Domaenen in einem
//! Lebenszyklus, und der falsche hat gewonnen (ISA F13).
//!
//! Die Doktrin nennt das Nomen: *"Capability owns a bounded domain, external system or data
//! store"* (CONTRIBUTING.md#three-architectural-nouns). Ein Datenbestand ist kein `libs/`, weil
//! libs keine Domaene besitzen — und `libs/content-item` begruendet ausdruecklich, warum ein
//! geteilter **Store** nicht dasselbe ist wie ein geteilter Lesevertrag.
//!
//! ## Was hier liegt und was nicht
//!
//! Hier: die Zeilen, ihr Zustand ueber die Zeit, die Wunschliste, der Link-Intake, die
//! Ausruestungsfelder, die Kleidungsfelder. **Nicht** hier: `interior_placement` (wo ein Stueck
//! in einer Wohnung steht — das ist raeumlich), die Raeumungsregeln, die Layouts, die Geometrie.
//!
//! Die Grenze ist eine Frage: **braucht es einen Raum?** Wenn ja, gehoert es `interior`.

pub mod api;
pub mod budget;
pub mod import;
pub mod obsidian;
pub mod store;
// Der Intake: ein geteilter Link wird eine Wunschzeile. Liest nur, was eine Seite ueber sich
// selbst deklariert, und begruendet im Modulkopf, warum ein Preis aus dem Fliesstext keiner ist.
pub mod wunsch;
