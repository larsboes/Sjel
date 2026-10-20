// Batch Apple Vision text recognition, for rung 2 of the extraction ladder
// (PRD Q63 -> B30; the engine's measurement is upstreams.toml [auge], the OS
// framework it stands on is systems.toml [apple-vision]).
//
// Adopted verbatim from the implementation that produced the 2026-08-31
// measurement, so the engine recorded there and the engine Sjel runs are the
// same binary. Only this header is new.
//
// PROTOCOL. Reads one file path per line on stdin. Emits, per input, one line:
//
//     \x1e<path>\x1f<recognized text>\n
//
// One process reads many pages, which is the whole reason this is a batch tool:
// Vision loads its language assets once, and a per-page fork pays that cost per
// page. \x1e (record separator) and \x1f (unit separator) frame the record
// because recognized text contains newlines and tabs and paths may contain
// almost anything else. Output is flushed per record, so a caller may stream.
//
// TWO LIMITS THE CODE DOES NOT SHOW, and both decide caller behaviour:
//
//  1. An empty text field is AMBIGUOUS. The file could not be loaded, or the
//     page carried no text -- the two return the same empty string. A caller
//     must therefore treat an empty record as a failure of this rung rather
//     than as an empty page, because an empty body is indistinguishable from
//     "this page had nothing" once it is stored. libs/extraction/src/vision.rs
//     does that.
//  2. NSImage(contentsOfFile:) renders PAGE ONE of a PDF and nothing else.
//     Handing this a multi-page PDF silently reads the first page and drops the
//     rest. libs/extraction/src/vision.rs therefore reads pixels only and
//     refuses the PDF class outright, at any length -- it cannot tell a
//     one-page PDF from a book, and a rung that reads page one under a producer
//     claiming it read the document is the failure this ladder exists to
//     prevent. Rasterizing pages is a named follow-up, not a quiet truncation.
import Foundation
import Vision
import AppKit

let langs = ["de-DE", "en-US"]

func recognize(_ path: String) -> String {
    guard let img = NSImage(contentsOfFile: path),
          let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return "" }
    let req = VNRecognizeTextRequest()
    req.recognitionLevel = .accurate
    req.recognitionLanguages = langs
    req.usesLanguageCorrection = true
    do { try VNImageRequestHandler(cgImage: cg, options: [:]).perform([req]) } catch { return "" }
    return (req.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
}

while let line = readLine(strippingNewline: true) {
    if line.isEmpty { continue }
    let text = recognize(line)
    print("\u{1e}\(line)\u{1f}\(text)")
    fflush(stdout)
}
