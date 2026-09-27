# interior — Raumplanung

Misst Layouts gegen die Räumungsregeln einer Wohnung und sucht Positionen, die sie erfüllen.

```
interior model                     gemessener Raum, und was daran noch geraten ist
interior layouts                   Layouts auf der Platte
interior check <layout> [--json]   gegen rules.toml prüfen (Exit 1 bei hartem Verstoß)
interior toleranz <layout>         bis zu welchem Messfehler das Verdikt hält
interior einbringung <layout>      kommt jedes Stück durch die Tür und an seinen Platz
interior sonne <layout>            wann im Jahr welches Stück in der Sonne steht
interior deklaration               wer noch am Namen gemessen wird, und was es kostet
interior kaufen                    welcher Bedarf zuerst, und wann er erreicht ist
interior search <layout> --move a,b [--step 20] [--limit 6] [--band id=x0,x1,y0,y1] [--out name]
interior compose --pieces a,b,c    eine Wohnung von Grund auf stellen (Strahlsuche)
interior plan [layout...] --out f  Pläne als HTML mit Verdikt
interior inventory                 was da ist, was fehlt, und was das Fehlende kostet
interior import                    inventory/*.toml in die Tabellen (wiederholbar)
interior serve                     HTTP-API, Port aus service.toml
```

Die Oberfläche ist `/interior` in der Axon-Shell (`dashboard/src/routes/interior/`). Sie
**rendert und rechnet nicht**: jedes Verdikt, jede Korridorbreite und der Plan als SVG kommen
fertig aus dieser Capability. Eine zweite Auslegung einer Räumungsregel im Frontend wäre genau
die Drift, gegen die das hier existiert.

## Diese Capability kennt keine Wohnung

Jede Zahl kommt zur Laufzeit aus dem privaten Overlay, aufgelöst über
`sjel_config::overlay_data_dir("interior")`. Ohne `SJEL_PERSONAL_ROOT` bricht sie ab statt zu
raten: eine Planung gegen erfundene Maße wäre schlimmer als gar keine.

Fotos und RoomPlan-Aufnahmen sind ebenfalls privat, aber keine Maschinenmodell-Daten. Ihr
Verzeichnis kommt aus `assets_root` in der privaten `config/interior.json` und liegt auf dieser
Maschine im Obsidian-Projekt `Projects/Apartment/Assets/Interior/`. Fehlen Datei oder Schlüssel,
fällt der Prozess für Fixtures und ältere Overlays auf `data/interior/` zurück. Eine Datei, die
existiert, aber nicht lesbar ist, kein JSON-Objekt ist oder `assets_root` nicht als Pfad führt,
bricht ab (`model::assets_dir`).

Welche Wohnung gemeint ist, entscheidet `SJEL_INTERIOR_FLAT` oder `--flat`; liegt genau eine
unter `flats/`, ist es die. Liegen mehrere und ist keine gewählt, ist das ein **Fehler und keine
Vorauswahl** — ein stiller Standard ist der Weg, auf dem ein Plan der falschen Wohnung als der
richtige durchgeht.

`tests/containment.rs` ist die Bedingung dafür, dass das hier stehen darf: es liest `src/`,
`tests/` und die Dateien im Wurzelverzeichnis und fällt um, sobald eine Wandlänge oder ein
Wohnungsname darin auftaucht. Beim ersten Lauf fand es drei Lecks (PRD §8.3, B24).

## Wo die Zahlen herkommen

Zwei Orte, und die Grenze ist **hat die Zahl eine Begründung, die mitwandern muss** (PRD Q60).

**Zeilen** — das Inventar, in der geteilten SQLite-Datei unter `interior_`:

| | |
|---|---|
| `interior_item` | jedes Stück und jeder Bedarf: Maße, Preis in Cent, Herkunft, was es an Platz verlangt |
| `interior_item_state` | `owned` / `wanted` / `gone` **über die Zeit**. Ein Wunsch, der gekauft wird, ist zwei Zeilen und nicht eine überschriebene |
| `interior_placement` | wo ein Stück in einer Wohnung wirklich steht — im Unterschied zu einem Layout, das ein Vorschlag ist |

**Dateien** — die Geometrie, unter `<overlay>/data/interior/flats/<id>/`:

| | |
|---|---|
| `room.toml` | Polygon, Wände, Öffnungen, feste Einbauten, `[[routen]]` |
| `rules.toml` | Laufwege, Abstände, Lichtkorridor, die Regeltexte |
| `layouts/*.toml` | die Varianten, über die argumentiert wird |

`room.toml` trägt datierte Korrekturkommentare, und drei davon sind das Protokoll eines Fehlers,
der einen falschen Plan erzeugt hat. Eine Tabelle hat dafür keine Spalte, und ein Maß ohne seine
Korrekturgeschichte ist, wie derselbe Fehler zweimal gemacht wird.

`inventory/*.toml` im Overlay ist seit B25 die **Migrationsquelle** und keine zweite Wahrheit:
`interior import` liest sie in die Tabellen, wiederholbar und ohne dabei Zustandsgeschichte zu
erfinden. Nichts hier korrigiert Maße — die sind mit einem Bandmaß auf Papier entstanden.

Die privaten Asset-Unterordner sind `media/` und `captures/`. Die Datenbank und die TOML-Dateien
bleiben im Overlay, damit der Prüfer weiterhin ohne Obsidian-Index rechnen kann; nur die grossen
bzw. visuellen Belege liegen beim menschlichen Projekt.

## Die Musterwohnung

`tests/fixtures/overlay/` ist ein vollständiges `data/interior/` mit **ausgedachter** Geometrie.
Sie ist der Grund, warum diese Capability öffentlich stehen und trotzdem geprüft sein kann:
`tests/engine.rs` läuft überall gegen sie, `tests/live_parity.rs` prüft gegen die echte Wohnung
und überspringt sich selbst ohne Overlay.

Ihre `rules.toml` führt bewusst andere Schwellen als jede reale — `bett_zugang_zweite_seite`
steht dort auf 40. Wer diese Zahl im Code sucht, findet sie nicht: jede Schwelle wird über ihren
Namen aus der Datei gelesen, damit eine Regel, die einen Wert erfindet statt ihn nachzuschlagen,
beim Lesen auffällt.

Es sind **zwei** Wohnungen, und die zweite existiert wegen einer einzigen Eigenschaft:
`muster-nordkueche` stellt die Küchenzeile an die Nordwand, wird also von Süden angelaufen. Bis
2026-08-30 rechnete der Prüfer diese Zone immer nach Norden — richtig für genau eine Wohnung,
falsch für diese, und nichts hätte es gemeldet. Nebenbei sind zwei Wohnungen der Fall, in dem
`default_flat()` sich weigern muss zu raten.

```bash
SJEL_PERSONAL_ROOT=capabilities/interior/tests/fixtures/overlay \
SJEL_INTERIOR_FLAT=muster target/debug/interior check a-frei
```

## Was die Maschine tut

Zwei Primitive tragen die Arbeit: eine **exakte** euklidische Distanztransformation, die jedem
freien Punkt seinen Abstand zum nächsten belegten gibt, und eine **Maximum-Bottleneck-Suche**,
die die engste Stelle des breitesten Weges liefert. Ein Korridor der Breite W hat auf seiner
Mittellinie die Freiheit W/2 — die verdoppelte Engstelle ist deshalb die Breite, die ein Mensch
tatsächlich bekommt. Genähert statt exakt verschiebt Korridorbreiten um mehrere Zentimeter, und
genau daran entscheidet sich hier Bestehen oder Verstoß.

Weich warnt, hart blockiert. Es gibt absichtlich keinen Weg, ein Bestehen zu melden, solange ein
harter Verstoß offen ist.

## Herkunft

Bis 2026-08-30 TypeScript im privaten Overlay — der einzige Ausreißer gegenüber core Axon, das
durchgehend ein Rust-Workspace ist. Die Portierung ist gegen ein aufgezeichnetes Protokoll der
alten Engine abgesichert: zehn Layouts, jedes Verdikt, jede Korridorbreite, jeder
Katalogeintrag. Das Protokoll beschreibt **eine** Wohnung und liegt deshalb im Overlay neben
ihr, nicht hier.

Zwei Dinge wurden dabei gemessen statt vermutet:

- `widestPath` suchte das Maximum bei jeder Entnahme mit einem linearen Durchlauf über alle
  ~10.000 Rasterzellen — O(N²) je Route. Der Kommentar daneben behauptete, das schlage einen
  Heap „in both clarity and speed". Mit Heap: **67,7 ms → 2,7 ms**. In Rust: **0,18 ms**.
- Die Suche prüft seitdem **erschöpfend statt bis zum ersten Treffer**: 3,5 Mio. Kandidaten in
  rund 100 s, parallel über rayon. Vorher lieferte sie die erste zulässige Lösung, jetzt die
  beste von allen.

Nach core Axon verschoben am 2026-08-30 durch PRD Q59. Der Grund für die Overlay-Lage vom
2026-08-20 — *„die Seite, die sie ausliefert, bettet Fotos einer Wohnung ein"* — ist an dem Tag
gestorben, an dem die Seite eine HTTP-API wurde, die Medien erst auf Anfrage ausliefert.

## Was die Suche nicht kann

`wandkontakt_cm` bevorzugt Möbel mit einer Wand im Rücken, weil der Räumungsprüfer nur Abstände
kennt und einen Esstisch mitten im Raum für genauso richtig hält wie einen an der Wand. Das Maß
ist grob, und die Rangfolge bleibt eine Hilfe und kein Urteil.

Ein Stück, das **frei stehen soll**, wurde dafür bis 2026-08-31 bestraft: ein Regal quer im Raum
bekam 0 cm und sank. Es kann jetzt `raumtrenner = true` sagen und fällt dann aus der Wandsumme
heraus — nicht mit 0 cm, sondern gar nicht, damit es die Bewertung der übrigen Stücke weder hebt
noch senkt. Freiwillig wie jedes Feld aus Q61: wer nichts erklärt, wird weiter an der Wand
gemessen, also bewegt sich kein bestehender Rang, bis jemand eine Zeile füllt.

Das betrifft **nur die Rangfolge**. `raumtrenner` kann kein Layout bestehen oder durchfallen
lassen. `tests/search.rs` belegt es an zwei Brettern mit identischen Maßen an derselben Stelle,
von denen genau eines die Zeile führt — vorher hatte diese Datei überhaupt keinen Test.

## Was die Wohnung erklären muss

Zwei Dinge standen bis 2026-08-30 als Literale im Prüfer und beschrieben genau eine Wohnung
(PRD B26a). Sie stehen jetzt in `room.toml`:

```toml
[[fix_moebel]]
id = "kuechenzeile"
# seite ist Geometrie, abstand ist der NAME einer Schwelle in rules.toml — nicht die Zahl.
anlaufzone = { seite = "nord", abstand = "vor_kuechenzeile" }

[[routen]]
von  = "eingangstuer"
nach = "kuechenzeile"
```

Wegpunkte sind Öffnungen außer Fenstern und feste Möbel mit `anlaufzone`. Ein Name, den es nicht
gibt, ist ein **Fehler** und keine Route, die still ausfällt — die alte Fassung übersprang sie,
und der Bericht sah danach aus wie einer über eine Wohnung mit weniger Wegen. Keine `[[routen]]`
heißt: kein Weg wird gemessen. Das ist eine Aussage, keine Voreinstellung.

## Wer die Regeln besitzt

Bis 2026-08-31 stand hier, die Regel-**Kennungen** seien „noch nicht deklariert": `R1`…`R7` als
Literale im Code, ihre Texte in `rules.toml`. Das war zu freundlich formuliert. Die Datei führte
`[[regeln]]` mit `id`, `schwere` und `text`, `model::Rules` parste die Liste, und **nichts las
sie**: die Schwere stand an allen 21 Ausgabestellen im Code, und `clearance.rs` behauptete im
Modulkopf trotzdem „Schwere folgt `rules.toml`". Eine Wohnung konnte R3 auf `hart` setzen und
bekam weiterhin eine Warnung. Dazu meldete der Prüfer die Ausklappzone des Schlafsofas als
`couch_ausklappen`, während jede `rules.toml` sie als **R8** führt — zwei Namen für eine Regel,
und der Bericht nannte den, den die Wohnung nicht kennt.

Seit 2026-08-31 (PRD B31) schlägt der Prüfer nach, und der Abgleich läuft in beide Richtungen:

| | |
|---|---|
| **Hausregeln** — `R1 R2 R3 R4 R7 R8` | Schwere **und** Text kommen aus `rules.toml`. Fehlt die Kennung dort, bricht die Prüfung ab und nennt die, die es gibt. Der Text wandert in den Bericht |
| **Invarianten** — `kollision`, `raumgrenze`, `laufweg`, `zugang`, … | Stehen im Code und tragen keinen Text. Zwei Möbel können sich nicht überlappen, und keine Wohnung kann das erlauben. Sie deklarieren zu lassen hieße, jede Wohnung eine Invariante wiederholen zu lassen, die sie nicht ändern kann — und die erste vergessene wäre eine still abgeschaltete Prüfung |
| **Deklariert, aber nicht geprüft** | Kein Fehler, sondern ein Bericht: `CheckResult::nicht_geprueft`. Die reale Wohnung führt R5 (Blendung am Schreibtisch) und R6 (der Blick vom Eingang aufs Bett), und beides prüft hier niemand. „Bestanden" heißt ab hier „bestanden, gemessen an den Regeln, die gemessen wurden" |

`tests/rules.rs` ist der Beleg und nicht der Modulkopf: er kopiert die Musterwohnung, setzt R3 in
der Kopie auf `hart`, und **dasselbe Layout fällt durch**. Käme die Schwere je wieder aus dem
Code, bestünden beide Kopien und der Test fiele um.

## Was ein Stück von sich aus verlangt

Bis 2026-08-31 riet die Prüfung aus dem Namen: `bett*` war ein Bett, `schrank*` ein Schrank, und
mit der Vermutung kam jede Schwelle mit. Das ist einmal teuer danebengegangen — `^couch` fing
`couchtisch`, also wurde ein Couchtisch gegen die Regeln eines Sofas geprüft, gefunden erst, als
ein echter Esstisch dazukam.

Ein Stück kann jetzt selbst sagen, was es braucht (PRD Q61 / B26):

```toml
opens        = "sued"                     # welche Seite Türen/Schubladen braucht
open_clear   = 65                         # cm davor, am Stück gemessen, nicht geerbt
wall_ok      = false                      # diese Seite darf NICHT an eine Wand
expands      = { dir = "sued", to = 165 } # Gesamttiefe ausgeklappt, nicht der Zuwachs
access_sides = 1                          # ein Bett für eine Person braucht eine Seite
access_clear = 60
raumtrenner  = true                       # soll frei stehen; zählt nicht in die Wandsumme
```

`opens` und `expands.dir` gelten in der Ausrichtung des Stücks und drehen mit `rot` mit. Damit
sieht die Prüfung etwas, das die Namensfassung nicht sehen konnte: ein Schrank mit den Türen zur
Wand ist kein gedrehter Schrank, sondern ein unbenutzbarer.

**Wer nichts erklärt, wird weiter am Namen gemessen.** Das ist Absicht und kein halber Umbau:
42 Zeilen an einem Tag umzustellen wäre ein Stichtag, an dem sich Verdikte ändern, ohne dass
jemand die Zahlen dahinter geprüft hat. Die Musterwohnung erklärt vier Stücke und zwei bewusst
nicht, damit beide Wege geprüft bleiben.

## Was der offene Bedarf kostet

`GET /api/wishlist` legt zwei Zahlen nebeneinander, die aus **derselben Datei** kommen und
keine über HTTP: die Summe der `wanted`-Zeilen, und den Median des Monatssaldos aus
`finance_transaction_projection` über die letzten abgeschlossenen Monate (PRD B29).

Kein Budget — niemand hat eine Obergrenze gesetzt, und `budget.rs` erfindet keine. Der laufende
Kalendermonat fällt heraus, weil ein halber Monat mit ganzen Fixkosten keine Messung ist. Ist
der Median nicht positiv, gibt es **keine** Monatszahl: aus einem Saldo, aus dem nichts übrig
bleibt, lässt sich nichts ansparen, und „−3,2 Monate" wäre eine Rechnung, die so tut, als
beantworte sie etwas.

## Um wie viel, nicht ob

Ein Verdikt ist ein Bit, und ein Bit beantwortet die Frage nicht, die beim Möbelkauf ansteht:
besteht dieses Layout mit 2 cm oder mit 40? Bis zum 2026-08-31 waren beide dasselbe Wort.

Jede Stelle, die eine Zahl gegen eine Schwelle hält, legt jetzt eine **Reserve** ab — ob sie
auslöst oder nicht. `engste_reserve_cm` ist die knappste harte davon: **um so viel besteht
dieses Layout.** Negativ heißt, um so viel es durchfällt.

Zwei Feinheiten, beide teuer gelernt und beide am selben Tag:

- `free_depth_on_side` deckelt auf die geforderte Tiefe. Richtig für ein Verdikt, ruinös für
  eine Reserve: die knappste und die großzügigste Aufstellung meldeten beide genau die
  Schwelle. Gemessen wird deshalb bis `RESERVE_HORIZONT` darüber hinaus, und wo der Deckel
  greift, sagt die Reserve `gedeckelt` — es ist **mindestens** so viel, nicht genau so viel.
- Die Raumgrenze misst mit und **zählt nicht**. Ein Schrank an der Wand hat einen Zentimeter
  Abstand zu ihr, weil er dort hingehört. Eine Stunde lang meldete jedes Layout der echten
  Wohnung „1 cm Reserve"; das Feld `bindend` ist die Trennung zwischen einer Enge und einem
  Rücken an der Wand. Ein negativer Wert zählt weiter — herausragen ist ein Verstoß.

## Was das Verdikt aushält

`interior toleranz <layout>` beantwortet dieselbe Frage in der Einheit, in der das Bandmaß
sich irrt: *bis zu welchem Messfehler hält das?* Variiert werden genau die Maße, die das
Inventar als geschätzt führt (`unsicher = ["h"]`), und nur nach oben — ein zu klein
geschätztes Maß ruiniert einen Umzug, ein zu groß geschätztes verschenkt Platz.

Gesucht wird mit einer Halbierung, was voraussetzt, dass Bestehen monoton fällt. Das ist
Geometrie und keine Annahme: ein größeres Rechteck belegt mehr Raster, also kann jede freie
Tiefe und jede Korridorbreite nur kleiner werden. `tests/toleranz.rs` prüft die Monotonie an
den Zahlen, statt den Kommentar zu glauben.

Nicht variiert wird die **Wohnung**: `room.toml` trägt ihre Unsicherheit als datierte
Korrekturkommentare, und das ist Prosa für einen Menschen (PRD Q60). Ein `toleranz_cm` in die
Geometrie zu erfinden wäre eine Zahl, die niemand gemessen hat. Der Bericht nennt das, statt
die Wohnung stillschweigend als exakt zu behandeln.

## Kommt es überhaupt herein

Jede andere Prüfung fragt, ob ein Möbel an einer Stelle **stehen** darf. Keine fragte, ob es
dorthin **gelangt** — und ein Schrank, der an seinem Platz jede Regel erfüllt und nicht durch
die Wohnungstür passt, ist gekauftes Geld.

`interior einbringung <layout>` sucht einen Weg im Konfigurationsraum: nicht wo das Möbel
steht, sondern jede Lage, die es einnehmen kann, Position und Drehung zugleich. Zwei Drehungen
genügen, weil ein Rechteck nach 180 Grad aussieht wie vorher.

Drei Vereinfachungen, alle in dieselbe Richtung, damit ein Ja belastbar ist:

| | |
|---|---|
| Gedreht wird nur, wo der **Umkreis** frei ist | Ein Mensch kippt einen Schrank auf die Ecke. Diese Maschine verlangt den ganzen Kreis — ein „geht nicht" heißt also „nicht ohne Kippen" |
| Hindernisse sind nur die **festen Einbauten** | Am Einzugstag ist die Wohnung leer, und in welcher Reihenfolge die übrigen Stücke hineingehen, entscheidet diese Datei nicht |
| Die **Türhöhe** wird nicht geprüft | `room.toml` führt Öffnungen mit `breite` und ohne Höhe. Eine Höhe zu erfinden wäre schlimmer als die Lücke zu nennen |

Ein 140 cm breites Bett passt durch keine 100 cm breite Tür und steht trotzdem in jedem
Schlafzimmer: es kommt in Teilen herein. Das sagt die **Zeile** und nicht eine Vermutung über
Betten — `zerlegbar = true`, freiwillig wie jedes Feld aus PRD Q61. Wer nichts sagt, wird als
ein Stück getragen; das ist die vorsichtige Richtung, denn eine falsche Warnung kostet ein
Nachdenken und eine fehlende den Schrank.

`tests/fixtures/.../k-vitrine.toml` ist der Fall, um den es geht: ein Layout, das **jede
Räumungsregel erfüllt** und trotzdem nie stattfindet, weil das Möbel 10 cm breiter ist als
die Tür. Bis 2026-08-31 war die Antwort darauf `BESTANDEN` — richtig gerechnet, falsche Frage.

## Wo die Sonne hinfällt

R5 fragt, auf welcher Achse der Schreibtisch zur Verglasung steht. Das kommt ohne Ort, Datum
und Uhrzeit aus und kann deshalb nicht beantworten, ob im März um neun die Sonne auf dem
Bildschirm steht.

`interior sonne <layout>` rechnet das: Azimut und Höhe nach dem Verfahren der NOAA, und daraus
den Lichtfleck, den eine Verglasung auf den Boden zeichnet — ein Parallelogramm mit dem Versatz
`hoehe / tan(sonnenhoehe)`. Vier Tage tragen das ganze Jahr: die Sonnenwenden sind die Extreme,
die Tagundnachtgleichen die Mitte.

Dafür muss die Wohnung drei Dinge sagen, und schweigt sie, wird **nicht gerechnet**:

```toml
[lage]
breite = 50.7            # Grad Nord
laenge = 7.1             # Grad Ost
utc_offset_h = 2         # die Zeitzone MIT Sommerzeit
nordrichtung_grad = 0    # welche Kompassrichtung im Plan nach oben zeigt

[[oeffnungen]]
glas_von_cm = 0          # Unterkante ueber dem Boden
glas_bis_cm = 210        # Oberkante
```

**Die reale Wohnung führt keines davon**, also läuft dort nichts und der Bericht sagt, was
fehlt. Die Musterwohnung führt alles und ist deshalb der Ort, an dem die Rechnung geprüft
ist. Geprüft wird sie gegen **Eigenschaften** und nicht gegen eine Tabelle: die Mittagshöhe
ist 90 Grad minus dem Abstand zwischen Breite und Deklination, und die Deklination erreicht an
den Sonnenwenden die Schiefe der Ekliptik. Eine abgeschriebene Referenztabelle prüft, ob
jemand richtig abgeschrieben hat.

Das Modell ist der **ungehinderte** Wurf: was zwischen Fenster und Boden steht, wirft keinen
Schatten, und Nachbarhäuser und Balkonplatten kennt es nicht. Es meldet also eher zu viel
Sonne als zu wenig — die vorsichtige Richtung für eine Blendungsfrage.

**R9** ist die erste Regel dieser Maschine mit einem Datum, und sie ist **optional**
(`clearance::OPTIONALE_REGEL_IDS`): eine Wohnung, die sie nicht führt, bricht nicht ab,
sondern schweigt. Führt sie sie ohne die Angaben, ist das eine Lücke und steht in
`nicht_geprueft` — dieselbe Unterscheidung, die B31 für R5 und R6 eingeführt hat.

## Vier Ziele, und keines gewinnt

Die Suche rankte bis 2026-08-31 nach `wandkontakt_cm`, mit dem Engpass als Nachrang. Das ist
eine Gewichtung, die niemand gemessen hat. Sie liefert jetzt zusätzlich die **Pareto-Front**
über vier Ziele — knappste Reserve, Wandkontakt, engster Weg, Zahl der Warnungen —, und die
Front steht vorn: ein dominierter Treffer ist unter JEDER Gewichtung schlechter als ein
bestimmter anderer, also kann er die Antwort nicht sein. Das ist Arithmetik und kein Geschmack.

Die Rangfolge innerhalb der Front ist unverändert die vom 2026-08-30. Die Reserve steht als
**letzte** Stufe in der Kette, und das ist der Unterschied zwischen einer neuen Rangfolge und
einer widerspruchsfreien: erst damit ist die Ordnung lexikographisch über alle vier Ziele, und
kein Treffer kann mehr vor einem stehen, der ihn in allem schlägt. Ein Test hält das fest —
ohne diese Stufe fand er zwei Aufstellungen, die sich um einen Zentimeter Reserve unterschieden
und in der falschen Reihenfolge standen.

## Die teuerste Rechnung, endlich erreichbar

`search` prüft erschöpfend und rechnet Minuten. Eine HTTP-Anfrage, die so lange offen steht,
ist eine Wette auf jeden Proxy dazwischen — und deshalb gab es die Suche bis 2026-08-31 nur auf
der Kommandozeile: die teuerste Rechnung dieser Capability war von der Oberfläche aus, die sie
braucht, nicht erreichbar.

`POST /api/search` und `POST /api/compose` antworten mit **202 und einer Auftragsnummer**,
`GET /api/auftraege/:id` sagt, was daraus geworden ist. Der Auftrag ist absichtlich das
kleinste, was funktioniert: eine Nummer, ein Zustand, ein Ergebnis. Er lebt im Prozess und
stirbt mit ihm, und das ist richtig — sein Ergebnis ist eine Liste von **Vorschlägen** und
keine Tatsache über die Wohnung. Wer einen behalten will, schreibt ihn als Layout.

Die Karte hält zehn Aufträge, und verdrängt wird nur, was schon **fertig** ist. Bis 2026-08-31
fiel der älteste heraus, gleich ob er noch rechnete: sein Hintergrundfaden schrieb das Ergebnis
dann in eine Nummer, die es nicht mehr gab, und der Abholer bekam **404** auf eine Suche, die
Minuten gelaufen war. Rechnen alle zehn, antwortet die elfte Anfrage seitdem **503** — eine
Absage ist ehrlicher als ein still verlorenes Ergebnis.

`limit` steht im Rumpf auf derselben Zahl wie auf der Kommandozeile: sechs für `search`, fünf
für `compose`. Bis 2026-08-31 fehlte dieser Wert für `search`, und die eingesetzte Null heißt
im Prüfer *unbegrenzt* — dieselbe Frage lieferte über HTTP Tausende Treffer und im Terminal
sechs. Wer mehr will, sagt eine größere Zahl.

## Einen Plan anlegen, und keinen verlieren

Diese Capability konnte Layouts messen, verschieben, drehen und suchen. **Anlegen** konnte sie
bis 2026-08-31 nur nebenbei: `POST /api/layouts` verlangte eine fertige Aufstellung, also musste
jeder, der eine Variante durchspielen wollte, sich erst eine besorgen. Ein leerer Plan war kein
vorgesehener Zustand, und eine Kopie hieß, die Stücke einmal herunterzuladen und wieder
hochzuschicken.

Es sind drei Anfänge, und keiner verdient eine eigene Route:

| | |
|---|---|
| `{id}` | ein **leerer** Plan — der Anfang jeder Planung, die von Hand gezogen wird |
| `{id, von}` | eine **Kopie**. Die Vorlage wird gelesen und nicht angefasst |
| `{id, items}` | eine **fertige** Aufstellung, wie die Oberfläche sie nach einem Zug schickt |

Dazu freiwillig `name` (sonst die Id) und `notiz`. Geantwortet wird mit `{layout, check, svg}` wie
bei `PUT` und der Vorschau: wer einen Plan anlegt, will als Nächstes wissen, ob er besteht und wie
er aussieht, und das ist dieselbe Rechnung.

Der Dateikopf war dabei ein Defekt. `layout_io::create` schrieb in **jede** neue Datei „Von einer
Maschine gesetzt … jede Position hat die volle Räumungsprüfung durchlaufen". Für `interior
compose` stimmt der Satz. Für einen leeren Plan, den jemand über die API anlegt und danach von
Hand zieht, ist er erfunden — und eine Datei, die ihre eigene Herkunft falsch behauptet, ist
schlimmer als eine ohne Kopf, weil sie geglaubt wird. Der Satz steht jetzt beim Aufrufer, der ihn
verantworten kann; ohne `notiz` schreibt der Kopf das Datum und die API hin und sonst nichts.

`DELETE /api/layouts/:name` **archiviert**. Die Datei wandert nach `layouts/archiv/` und fällt aus
`Model::layout_names` heraus, weil ein Verzeichnis keine `.toml`-Endung trägt. Gelöscht wird
nichts: `h-esstisch-offen` trägt drei Absätze darüber, warum eine Klappe nicht aufgeht, und genau
dieses Argument wird gebraucht, wenn derselbe Tisch wieder zur Debatte steht. Ein `DELETE`, das
die Datei entfernt, entfernt die Begründung mit.

## Der Plan zeichnete die Möbel über die Wände

Ein Plan, dessen Zahlen stimmen, kann trotzdem falsch aussehen, und dann wird ihm nicht geglaubt.
Bis 2026-08-31 zeichnete `plan.rs` die Wände **vor** den Möbeln. Eine Kontur mit `stroke-width`
liegt mittig auf ihrer Linie, also deckte jedes Stück, das an einer Wand steht, die halbe
Wandstärke zu — im Bild verschwand die Wand hinter dem Schrank, und der Schrank sah aus, als
stünde er halb außerhalb der Wohnung. Die geprüfte Geometrie war die ganze Zeit richtig; kein
Layout meldete `raumgrenze`.

Wand und Öffnung werden jetzt zuletzt gezeichnet. Die Öffnung gehört aus einem zweiten Grund nach
oben: **dass ein Schrank vor der Tür steht, ist genau die Auskunft, für die jemand den Plan
ansieht**, und darunter war sie unsichtbar. Dazu stand die Beschriftung eines festen Einbaus 40 cm
nach Süden versetzt und trug für jeden Einbau das Wort `KUECHE`; bei einer Küchenzeile an der
Südwand fiel sie damit aus der Wohnung heraus. Sie steht jetzt mittig und nennt die `id` aus der
Datei.

**Nicht** geändert wurde der Ausschnitt. Er umfasst Bad und Terrasse mit, und wo die Terrasse
`geschätzt = true` führt, bekommt die gemessene Wohnung dadurch weniger als die Hälfte der
Bildbreite. Das ist eine Aussage über die Daten und kein Zeichenfehler: die Terrassentür führt
dorthin, und der Ausschnitt eine erfundene Zahl kleiner zu rechnen wäre schlimmer als ein Plan mit
Luft daneben.

Die Oberfläche zeigt seitdem beim Überfahren eines Stücks seine Maße — die Grundfläche **wie
gezeichnet**, denn eine Drehung um 90 Grad vertauscht b und t, und `footprint` ist das, was sie
vertauscht. Sie liest sie deshalb aus der Zeichnung zurück, statt sie nachzurechnen. Ebenfalls
korrigiert: der Ziehe-Editor baute jeden Eintrag beim Speichern neu aus dem SVG auf und verlor
dabei `size`, die Angabe, dass ein Tisch **aufgeklappt** ist. Acht Layouts dieser Wohnung führen
eine, also setzte jedes Verschieben eines Stücks ein anderes still auf sein Katalogmaß zurück.

## Was zuerst gekauft wird

`interior kaufen` ordnet den offenen Bedarf: Dringlichkeit, dann ob ein Entwurf schon darauf
baut, dann der Preis. Der Preis steht **zuletzt** — er entscheidet zwischen gleich dringenden
Posten und nicht darüber, was dringend ist.

Ein Wort, zwei Achsen, und beide stehen wirklich in den Daten: ein `[[slot]]` sagt, wie dringend
das Bedürfnis ist (`pflicht`, `empfehlung`, `konzept`), ein `[[produkt]]`, wie weit die
Entscheidung ist (`gesetzt`, `kandidat`, `zurueckgestellt`, `verworfen`, `ersetzt`). Die Liste
in `budget::BEKANNTE_PRIORITAETEN` ist aus dem Inventar **gelesen** und nicht aus dem PRD
abgeschrieben: der erste Entwurf kannte drei der acht Wörter, die übrigen fielen still auf
denselben Rang, und was zwischen einem *gesetzten* und einem *zurückgestellten* Produkt
entschied, war der Preis. Ein unbekanntes Wort steht seitdem hinten **und wird gemeldet**.

Kein Budget, keine Empfehlung. Die Zeitachse kommt aus derselben Messung wie B29 — dem Median
des Monatssaldos —, und ist er nicht positiv, gibt es keine Monatszahl. „Nicht gemessen" und
„daraus lässt sich nichts ansparen" sind dabei zwei verschiedene Sätze und bekommen zwei
verschiedene Wörter.

## Wie weit das Inventar der Maschine hinterherhängt

Q61 hat die Namensheuristik abgelöst, B26 hat sie absichtlich stehen lassen. Die Folge war ein
Stillstand: die Maschine ist klüger als ihre Daten, und niemand konnte sehen, wie viel klüger.

`interior deklaration` beantwortet das je Eintrag in drei Zahlen — erklärt er sich schon,
wofür hält die Heuristik ihn sonst, und **was ändert sich, wenn er sich erklärt**. Der
Vorschlag schreibt auf, was die Maschine ohnehin annimmt, als TOML zum Einfügen; die Folgen
sind dieselbe Rechnung, die `POST /api/items/:id/impact` für jede andere Änderung führt.

Er ist **keine Empfehlung und nicht wirkungsfrei**: die Namensfassung prüft an einem Bett die
beiden Längsseiten, die deklarierte Fassung zählt jede Seite, die tief genug ist. Das ist
nicht dieselbe Frage, und deshalb steht bei jedem Eintrag, was sich bewegt. Wer die Zeile
übernimmt, übernimmt eine Entscheidung — und darum schreibt diese Maschine nichts.

## Gleichzeitiges Bearbeiten

Telefon und Mac bearbeiten dieselben Einträge (PRD §10 A5, Q110). Bis 2026-09-24 hatten
`PUT` und `PATCH /api/items/:id` keine Versionsprüfung, und der spätere Schreiber gewann still.

Jeder Eintrag trägt deshalb `revision`, eine ganze Zahl. Bestehende Zeilen beginnen bei 1, jedes
Schreiben erhöht sie im selben Statement, auch der Import. Sie steht in jeder Antwort, die einen
Eintrag zeigt (`/api/inventory`, `/api/wishlist`), und nach jedem Schreiben im
Rumpf und als `ETag`. Sie gehört dem Server: ein `revision` im Rumpf wird nie geschrieben.

Wer gegen den gelesenen Stand schreiben will, schickt die Revision mit:

| | |
|---|---|
| `If-Match: "3"` oder `If-Match: 3` | der Hauptweg, wie ein ETag |
| `"expected_revision": 3` im Rumpf | Ersatz für einen Client ohne eigene Header |

Nennen beide eine Zahl und nicht dieselbe, antwortet der Server 400.

| Fall | Antwort |
|---|---|
| Revision passt | 200, `{id, ok, revision}` mit der neuen Revision |
| Revision veraltet | 409, `{error, current: {item, state}}` mit dem Stand, der jetzt gilt. Geschrieben wird nichts |
| Eintrag fehlt | 404, bei `PUT` mit Revision auch: angelegt wird nur ohne Bedingung |
| keine Revision | wie vor A5, der spätere Schreiber gewinnt |

Vergleich und Schreiben sind ein Statement, `UPDATE … WHERE id = ? AND revision = ?`
(`store::Store::update_item_if_revision`). Erst wenn es keine Zeile trifft, liest der Server
nach, ob der Eintrag fehlt oder veraltet ist. Von zwei Schreibern mit derselben Revision gewinnt
genau einer; `tests/revision.rs` prüft das mit echten Threads gegen eine SQLite-Datei.

**Ein Client, der offline sein kann, muss die Revision schicken.** Ohne sie überschreibt er
beim Wiederverbinden alles, was inzwischen auf einem anderen Gerät geschah. Auf 409 zeigt er den
aktuellen Stand und lässt den Menschen entscheiden; ein automatisches Wiederholen mit der neuen
Revision wäre dasselbe stille Überschreiben, nur einen Umweg später. Das Dashboard hält sich
daran (`dashboard/src/lib/api.ts`, `InteriorConflict`).
