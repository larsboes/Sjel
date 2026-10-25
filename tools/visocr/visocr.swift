// Batch Apple Vision text recognition, for rung 2 of the extraction ladder
// (PRD Q63 -> B30; the engine's measurement is upstreams.toml [auge], the OS
// framework it stands on is systems.toml [apple-vision]).
//
// Upright recognition runs first; orthogonal orientations are tried only when it
// returns no text, which keeps the common printed-page path to one Vision pass.
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
//  2. A path ending in .pdf is rendered page by page through PDFKit and every
//     page is read; pages are joined with \x0c (form feed). NSImage alone would
//     render PAGE ONE of a PDF and drop the rest, which is why the PDF branch
//     exists. Any other path is read as one image.
import Foundation
import Vision
import AppKit
import PDFKit
import ImageIO

let langs = ["de-DE", "en-US"]

func recognize(_ path: String) -> String {
    if path.lowercased().hasSuffix(".pdf") { return recognizePDF(path) }
    guard let img = NSImage(contentsOfFile: path),
          let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return "" }
    return recognize(cg)
}

// 3x the PDF point size is 216 dpi, enough for Vision on printed A4.
func recognizePDF(_ path: String) -> String {
    guard let doc = PDFDocument(url: URL(fileURLWithPath: path)) else { return "" }
    var pages: [String] = []
    for index in 0..<doc.pageCount {
        guard let page = doc.page(at: index) else { pages.append(""); continue }
        let box = page.bounds(for: .mediaBox).size
        let image = page.thumbnail(of: NSSize(width: box.width * 3, height: box.height * 3), for: .mediaBox)
        guard let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { pages.append(""); continue }
        pages.append(recognize(cg))
    }
    return pages.joined(separator: "\u{0c}")
}

func recognize(_ cg: CGImage) -> String {
    let upright = recognize(cg, orientation: .up)
    if !upright.isEmpty { return upright }

    var best = ""
    for orientation in [CGImagePropertyOrientation.right, .left, .down] {
        let candidate = recognize(cg, orientation: orientation)
        if isBetter(candidate, than: best) { best = candidate }
    }
    return best
}

func recognize(_ cg: CGImage, orientation: CGImagePropertyOrientation) -> String {
    let req = VNRecognizeTextRequest()
    req.recognitionLevel = .accurate
    req.recognitionLanguages = langs
    req.usesLanguageCorrection = true
    do {
        try VNImageRequestHandler(cgImage: cg, orientation: orientation, options: [:]).perform([req])
    } catch {
        return ""
    }
    return (req.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
}

func isBetter(_ candidate: String, than current: String) -> Bool {
    let candidateLines = candidate.split(whereSeparator: { $0.isNewline }).count
    let currentLines = current.split(whereSeparator: { $0.isNewline }).count
    return candidateLines > currentLines ||
        (candidateLines == currentLines && candidate.count > current.count)
}

while let line = readLine(strippingNewline: true) {
    if line.isEmpty { continue }
    let text = recognize(line)
    print("\u{1e}\(line)\u{1f}\(text)")
    fflush(stdout)
}
