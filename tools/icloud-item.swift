import Foundation

// The filesystem can report the size of an offloaded item without holding its bytes.
// Query the specific iCloud item; a directory listing is not an upload acknowledgement.
let arguments = CommandLine.arguments
if arguments.count != 4 || !["wait-upload", "wait-download", "check-upload"].contains(arguments[1]) {
    fputs("usage: icloud-item.swift <wait-upload|wait-download|check-upload> <path> <seconds>\n", stderr)
    exit(2)
}
let mode = arguments[1]
var url = URL(fileURLWithPath: arguments[2])
guard let deadline = TimeInterval(arguments[3]), deadline >= 0 else {
    fputs("icloud-item: invalid timeout\n", stderr)
    exit(2)
}
let keys: Set<URLResourceKey> = [
    .isUbiquitousItemKey,
    .ubiquitousItemIsUploadedKey,
    .ubiquitousItemUploadingErrorKey,
    .ubiquitousItemDownloadingStatusKey,
]
let started = Date()
var requestedDownload = false
var lastUploadingError: String? = nil

while true {
    do {
        url.removeAllCachedResourceValues()
        let state = try url.resourceValues(forKeys: keys)
        guard state.isUbiquitousItem == true else {
            fputs("icloud-item: path is not an iCloud item\n", stderr)
            exit(1)
        }
        if mode == "wait-download" {
            if state.ubiquitousItemDownloadingStatus == .current { exit(0) }
            if !requestedDownload {
                try FileManager.default.startDownloadingUbiquitousItem(at: url)
                requestedDownload = true
            }
        } else {
            if state.ubiquitousItemIsUploaded == true { exit(0) }
            if let error = state.ubiquitousItemUploadingError {
                if mode == "check-upload" {
                    fputs("icloud-item: upload failed: \(error.localizedDescription)\n", stderr)
                    exit(1)
                }
                lastUploadingError = error.localizedDescription
            }
        }
    } catch {
        fputs("icloud-item: status unavailable: \(error.localizedDescription)\n", stderr)
        exit(1)
    }
    if mode == "check-upload" || Date().timeIntervalSince(started) >= deadline {
        if let error = lastUploadingError {
            fputs("icloud-item: upload failed: \(error)\n", stderr)
        } else {
            fputs("icloud-item: upload or download not complete within deadline\n", stderr)
        }
        exit(1)
    }
    Thread.sleep(forTimeInterval: 5)
}
