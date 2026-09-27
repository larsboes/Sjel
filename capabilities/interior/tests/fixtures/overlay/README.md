# Die Musterwohnung — erfundene Geometrie, damit die Maschine ohne Overlay laeuft

Ein vollstaendiges `data/interior/` mit **ausgedachten** Zahlen. Kein Mass hier ist gemessen,
und keins beschreibt eine Wohnung, die es gibt.

Warum die Datei so aussieht wie ein Overlay und nicht wie eine Testhilfe: die Capability loest
ihre Daten ueber `sjel_config::overlay_data_dir` auf, also ueber `SJEL_PERSONAL_ROOT`. Ein Test,
der stattdessen einen Testpfad in `src/` einschleust, prueft einen Codeweg, den kein Deployment
je nimmt. Die Tests setzen deshalb `SJEL_PERSONAL_ROOT` auf dieses Verzeichnis und laufen durch
dieselbe Aufloesung wie die echte Installation — `src/` bekommt keine Testabzweigung.

Die echte Wohnung liegt im privaten Overlay. `tests/live_parity.rs` prueft gegen sie und
ueberspringt sich selbst, wenn `SJEL_PERSONAL_ROOT` nicht gesetzt ist — das ist der Zustand in
CI und auf jedem Rechner, der die Dateien nicht hat.
