// Frozen corpus page: a German journey table. Rendered to de-table.png.
//
// The shape upstreams.toml [auge] rejected auge on: Vision returns text
// observations, so a table comes back COLUMN-MAJOR with the legs interleaved.
// That is a layout loss, not a character loss, which is why this page's
// judgements ask only that every cell SURVIVE. Whether the rows can be
// reconstructed is a different question, and no acceptance threshold here
// pretends to answer it.
#set page(width: 170mm, height: auto, margin: 14mm, fill: white)
#set text(size: 10pt, lang: "de")

= Ihre Verbindung

#table(
  columns: 6,
  align: left,
  table.header[Datum][Ab][Bahnhof][An][Bahnhof][Zug],
  [29.08.], [08:14], [Köln Hbf], [11:05], [Frankfurt(M) Hbf], [ICE 622],
  [29.08.], [11:32], [Frankfurt(M) Hbf], [14:47], [München Hbf], [ICE 519],
)
