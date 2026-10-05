pub mod api;
pub mod budget;
pub mod clearance;
pub mod deklaration;
pub mod einbringung;
pub mod geometry;
pub mod layout_io;
pub mod model;
pub mod plan;
// Die Bruecke von der rohen RoomPlan-USDZ nach Zentimeter. Liest, rechnet, schreibt nichts
// zurueck — der Modulkopf begruendet, warum das die Bedingung dafuer ist, dass ein Scan ein
// Beleg bleiben darf.
pub mod roomplan;
pub mod search;
pub mod sonne;
// Liest die Zeilen aus `capabilities/inventory` und schreibt sie nie: was hier bleibt, ist
// `interior_placement` (wo ein Stueck steht — das ist raeumlich) und der Lesepfad, ohne den
// kein Layout zu rechnen ist. Die Schreibhaelfte, `import`, `wunsch` und die Vault-Bruecke
// sind am 2026-10-05 mit der Item-Oberflaeche nach `capabilities/inventory` gezogen (ISA F13).
pub mod store;
pub mod toleranz;
