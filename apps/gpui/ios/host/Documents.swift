import Foundation
import PDFKit
import UIKit

@_cdecl("sjel_ios_documents_pdf")
public func sjelIOSDocumentsPdf(_ path: UnsafePointer<CChar>) {
    let url = URL(fileURLWithPath: String(cString: path))
    DispatchQueue.main.async { openPDF(url) }
}

private func openPDF(_ url: URL) {
    guard let host = activeGpuiController(for: .documents),
          host.presentedViewController == nil else { return }
    guard let document = PDFDocument(url: url) else {
        let alert = UIAlertController(title: "Could not open the original",
                                      message: nil, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        host.present(alert, animated: true)
        return
    }
    let view = PDFView()
    view.autoScales = true
    view.displayMode = .singlePageContinuous
    view.displayDirection = .vertical
    view.backgroundColor = .systemBackground
    view.document = document
    let reader = UIViewController()
    reader.view = view
    reader.title = "Original"
    reader.navigationItem.rightBarButtonItem = UIBarButtonItem(
        systemItem: .done, primaryAction: UIAction { [weak reader] _ in
            reader?.dismiss(animated: true)
        })
    host.present(UINavigationController(rootViewController: reader), animated: true)
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
