// Frozen corpus page: English prose. Rendered to en-prose.png by render.sh.
// The second half of the DE/EN balance the relevance corpus set as the house
// rule (capabilities/comms/eval/README.md, "Why this shape").
#set page(width: 150mm, height: auto, margin: 14mm, fill: white)
#set text(size: 11pt, lang: "en")
#set par(justify: true)
// No hyphenation: a hyphen inserted at a line break is a real `-` in the
// recognized text, and this corpus judges an engine on the characters it read,
// not on where the renderer chose to split a word.
#set text(hyphenate: false)

= Reading a document is not cleaning it

An extractor answers one question: what does this document say. It does
not decide which parts of the page are worth keeping. That judgement
belongs to a later stage, where it can be inspected and argued with,
rather than being folded silently into the reader.

The distinction matters because the two failures look nothing alike. A
reader that drops a paragraph has lost evidence. A cleaner that keeps a
navigation bar has kept noise. Only the first one is expensive, and only
the first one is invisible once the text has been stored.
