// Frozen corpus page: German and English prose on one page. Rendered to
// de-en-mixed.png.
//
// Here because visocr passes both languages in one request
// (`["de-DE", "en-US"]`, tools/visocr/visocr.swift) and a single-language
// engine can look perfect on the two monolingual pages while degrading the
// moment both appear together. Nothing else in this corpus would notice.
#set page(width: 150mm, height: auto, margin: 14mm, fill: white)
#set text(size: 11pt)
#set par(justify: true)
// No hyphenation: a hyphen inserted at a line break is a real `-` in the
// recognized text, and this corpus judges an engine on the characters it read,
// not on where the renderer chose to split a word.
#set text(hyphenate: false)

= Hinweis für Reisende / Notice to passengers

#text(lang: "de")[
  Der Aufzug zum Bahnsteig sieben ist wegen Wartungsarbeiten bis
  Freitag gesperrt. Bitte benutzen Sie die Rampe am nördlichen Ende der
  Halle. Für Rollstuhlfahrer steht ein Begleitdienst bereit.
]

#text(lang: "en")[
  The lift to platform seven is closed for maintenance until Friday.
  Please use the ramp at the northern end of the hall. Assistance for
  wheelchair users is available on request.
]
