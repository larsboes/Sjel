import Foundation
import PDFKit
import UIKit

// The views and folders behind the Documents reader (ios/src/nsh.rs, docs/plans/gpui.md §11).

/// A PDFKit view that fits the page width, for the `sjel.pdf` component. The caller owns the +1.
@_cdecl("sjel_ios_pdf_view")
public func sjelIOSPdfView() -> UnsafeMutableRawPointer {
    let view = PDFView()
    view.autoScales = true
    view.displayMode = .singlePageContinuous
    view.displayDirection = .vertical
    view.backgroundColor = .systemBackground
    return Unmanaged.passRetained(view).toOpaque()
}

/// Loads the PDF at `path` into a view `sjel_ios_pdf_view` made.
@_cdecl("sjel_ios_pdf_load")
public func sjelIOSPdfLoad(_ view: UnsafeMutableRawPointer, _ path: UnsafePointer<CChar>) {
    let pdfView = Unmanaged<PDFView>.fromOpaque(view).takeUnretainedValue()
    pdfView.document = PDFDocument(url: URL(fileURLWithPath: String(cString: path)))
}

/// Creates `path` as a folder whose new files are unreadable while the phone is locked
/// (`NSFileProtectionComplete`). Returns false when it cannot.
@_cdecl("sjel_ios_protected_folder")
public func sjelIOSProtectedFolder(_ path: UnsafePointer<CChar>) -> Bool {
    let folder = String(cString: path)
    do {
        try FileManager.default.createDirectory(
            atPath: folder, withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.complete])
        try FileManager.default.setAttributes(
            [.protectionKey: FileProtectionType.complete], ofItemAtPath: folder)
        return true
    } catch {
        return false
    }
}
