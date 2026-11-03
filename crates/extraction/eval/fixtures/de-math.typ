// Frozen corpus page: German physics notation. Rendered to de-math.png.
//
// This page reconstructs the recorded failure, deliberately: upstreams.toml
// [auge], 2026-08-31, records Apple Vision reading `=` as `-` on exactly this
// shape ("q- 10nC", "d-2am") and expressing no fraction at all. The quantities
// and the displayed fractions below are here so an engine either reproduces
// that failure or does not, on a page nobody had to keep a scan of.
#set page(width: 150mm, height: auto, margin: 14mm, fill: white)
#set text(size: 11pt, lang: "de")

= Elektrisches Feld einer Punktladung

Gegeben:

$ q = 10 "nC" $

$ d = 2 "cm" $

Die Feldstärke im Abstand $d$ folgt aus der Definition

$ E = F / q $

und mit dem Coulombgesetz

$ E = 1 / (4 pi epsilon_0) dot q / d^2 $

Die Arbeit längs eines Weges ist

$ W = integral_0^d F dif s $

und für eine Reihe von Ladungen gilt

$ E_"ges" = sum_(i=1)^n E_i $
