// Single CV template — no macro-conditional preamble, no per-target section files.
// Profile/language selection happens here as plain array filtering over the parsed master
// data; that's templating-layer work, not a separate compiled tool.
//
// Invoked as:
//   typst compile --root / --input master=<abs-path-to-master_cv.yaml> \
//     --input profile=<tag> --input lang=<en|de> templates/cv.typ <out.pdf>
// `--root /` lets this template read the master file from the selected overlay regardless of where
// that overlay actually lives on disk (its location is configurable, not guaranteed to be a
// sibling of this repo) — safe here since both template and data are fully trusted, authored
// by the same person running the compile locally.

#let profile = sys.inputs.at("profile", default: "default")
#let lang = sys.inputs.at("lang", default: "en")
#let data = yaml(sys.inputs.master)
// An application file (`cv app build`) may override `title` and `summary` with `{en, de}`.
#let app = if "app" in sys.inputs { yaml(sys.inputs.app) } else { (:) }

#let accent = rgb("#117A65")

#set document(title: data.name + " — CV")
#set page(paper: "a4", margin: (x: 1.8cm, y: 1.4cm))
#set text(font: "Helvetica Neue", size: 10pt, lang: lang)
#set par(leading: 0.55em, justify: false)

// true if an item (an experience/skills entry, or a bullet/detail within one) applies to the
// active profile — no `profiles` key (or an empty list) means "always included"
#let included(item) = {
  if "profiles" not in item { true }
  else if item.profiles.len() == 0 { true }
  else { profile in item.profiles }
}

// pick the `{en, de}` text variant for the active language. A bare string passes through
// unchanged, so a field may be plain where both languages share the text (an event name, a
// city) and a `{en, de}` map where they don't (a section's "since Jun 2025" / "seit Juni 2025")
// without the template needing to know which fields are which.
#let loc(variant) = if type(variant) == str { variant } else {
  variant.at(lang, default: variant.at("en"))
}

// pick the profile-specific override of a `{default: {en,de}, <profile>: {en,de}, ...}`
// text-variant map, falling back to `default`
#let variant-for(map) = loc(map.at(profile, default: map.default))

// The application's override of a top-level text-variant map, else the profile's variant.
#let text-for(key) = if key in app { loc(app.at(key)) } else { variant-for(data.at(key)) }

#let section-title(txt) = block(sticky: true)[
  #v(0.3em)
  #text(fill: accent, weight: "bold", size: 11.5pt, tracking: 0.5pt)[#upper(txt)]
  #v(-0.55em)
  #line(length: 100%, stroke: 1pt + accent)
  #v(0.15em)
]

// ---- Header ----

#align(center)[
  #text(size: 20pt, weight: "bold")[#data.name] \
  #text(size: 11.5pt, fill: accent, weight: "medium")[#text-for("title")]
]

#v(0.3em)

#align(center)[
  #text(size: 8.8pt)[
    #data.contact.location
    #if "phone" in data.contact [ #sym.bullet #data.contact.phone ]
    #sym.bullet #link("mailto:" + data.contact.email)[#data.contact.email]
    #sym.bullet #link("https://" + data.contact.linkedin)[#data.contact.linkedin]
    #sym.bullet #link("https://" + data.contact.github)[#data.contact.github]
  ]
]

#v(0.6em)

// ---- Summary ----

#text(size: 9.3pt)[#text-for("summary")]

// ---- Experience ----

#{
  let entries = data.experience.filter(included)
  if entries.len() > 0 {
    section-title(if lang == "de" { "Berufserfahrung" } else { "Experience" })
    for e in entries {
      let bullets = e.at("bullets", default: ()).filter(included)
      // An entry whose bullets were all filtered out is hidden. One with none is a header line.
      if bullets.len() > 0 or e.at("bullets", default: ()).len() == 0 {
        grid(
          columns: (1fr, auto),
          text(weight: "bold", size: 9.8pt)[
            #if "role" in e [#loc(e.role) #sym.dash.en ]#e.company #sym.dot.c #e.location
          ],
          text(size: 8.6pt, style: "italic")[#e.date],
        )
        for b in bullets {
          [#sym.bullet #loc(b.text)]
          linebreak()
        }
        v(0.3em)
      }
    }
  }
}

// ---- Education ----

#{
  let entries = data.education.filter(included)
  if entries.len() > 0 {
    section-title(if lang == "de" { "Ausbildung" } else { "Education" })
    for e in entries {
      grid(
        columns: (1fr, auto),
        text(weight: "bold", size: 9.8pt)[#e.institution #sym.dot.c #loc(e.degree)],
        text(size: 8.6pt, style: "italic")[#e.date],
      )
      for d in e.details {
        [#sym.bullet #loc(d)]
        linebreak()
      }
      v(0.3em)
    }
  }
}

// ---- Skills ----

#{
  let entries = data.skills.filter(included)
  if entries.len() > 0 {
    section-title(if lang == "de" { "Fähigkeiten" } else { "Skills" })
    for s in entries {
      // `items` is normally one list shared by both languages (tool names don't translate).
      // A `{en: [...], de: [...]}` map is the exception — spoken languages, where every item
      // differs — and is selected the same way as any other localized field.
      let items = if type(s.items) == dictionary { loc(s.items) } else { s.items }
      [#text(weight: "bold", size: 9pt)[#loc(s.category):] #items.join(", ")]
      linebreak()
    }
  }
}

// ---- Free sections ----
//
// Everything the four fixed sections above cannot express, as data rather than as
// template code. One generic shape instead of a block per section: the CV master
// carried Hackathons & Events, Soft Skills, Community & Engagement and Interests
// with no field to put them in, and hardcoding four more `#{ }` blocks would have
// meant a template edit every time a fifth appeared.
//
// A section renders whichever of the three content keys it has, in this order, and
// may carry all three. `entries` is the label/meta/detail row (a hackathon, a talk,
// an award); `bullets` is a plain list; `prose` is a paragraph. `profiles:` filters
// the section and each entry or bullet inside it, exactly like everywhere else.
#{
  for s in data.at("sections", default: ()).filter(included) {
    section-title(loc(s.title))
    if "intro" in s { [#loc(s.intro)]; v(0.35em) }
    for e in s.at("entries", default: ()).filter(included) {
      grid(
        columns: (1fr, auto),
        text(weight: "bold", size: 9.8pt)[#loc(e.label)],
        text(size: 8.6pt, style: "italic")[#loc(e.at("meta", default: ""))],
      )
      if "detail" in e { [#loc(e.detail)]; linebreak() }
      v(0.2em)
    }
    // `label` is bold, `text` is the rest. Two fields rather than markup inside one:
    // a YAML string interpolated into Typst renders literally, so "*Leadership*" in the
    // data would print its own asterisks — which is exactly what it did on first build.
    for b in s.at("bullets", default: ()).filter(included) {
      [#sym.bullet ]
      if "label" in b { [#text(weight: "bold")[#loc(b.label)] ] }
      [#loc(b.text)]
      linebreak()
    }
    if "prose" in s { [#loc(s.prose)] }
  }
}
