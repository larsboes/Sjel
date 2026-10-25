import AVFoundation
import CryptoKit
import Foundation
import QuickLook
import RoomPlan
import UIKit
import UniformTypeIdentifiers

private struct CaptureSource: Encodable {
    let platform = "ios"
    let roomplanVersion: String
    let appVersion: String?

    enum CodingKeys: String, CodingKey {
        case platform
        case roomplanVersion = "roomplan_version"
        case appVersion = "app_version"
    }
}

private struct CoordinateSystem: Encodable {
    let units = "meters"
    let upAxis = "Y"

    enum CodingKeys: String, CodingKey {
        case units
        case upAxis = "up_axis"
    }
}

private struct RoomElement: Encodable {
    let id: String
    let category: String
    let dimensionsM: [Float]
    let transform: [Float]
    let confidence: String
    let sourceID: String

    enum CodingKeys: String, CodingKey {
        case id, category, transform, confidence
        case dimensionsM = "dimensions_m"
        case sourceID = "source_id"
    }
}

private struct CapturedRoomPayload: Encodable {
    let id: String
    let surfaces: [RoomElement]
    let openings: [RoomElement]
    let objects: [RoomElement]
}

private struct CaptureAsset: Encodable {
    let assetID: String
    let role = "parametric"
    let format = "usdz"
    let byteLength: UInt64
    let sha256: String
    let storageToken: String

    enum CodingKeys: String, CodingKey {
        case role, format, sha256
        case assetID = "asset_id"
        case byteLength = "byte_length"
        case storageToken = "storage_token"
    }
}

private struct CaptureProvenance: Encodable {
    let captureMode = "new_room"
    let captureStartedAt: String
    let captureFinishedAt: String

    enum CodingKeys: String, CodingKey {
        case captureMode = "capture_mode"
        case captureStartedAt = "capture_started_at"
        case captureFinishedAt = "capture_finished_at"
    }
}

private struct UploadReceipt: Codable {
    let state: String
    let uploadedAt: String?

    enum CodingKeys: String, CodingKey {
        case state
        case uploadedAt = "uploaded_at"
    }
}

private final class RoomPlanPreviewController: QLPreviewController, QLPreviewControllerDataSource {
    private let assetURL: URL

    init(assetURL: URL) {
        self.assetURL = assetURL
        super.init(nibName: nil, bundle: nil)
        dataSource = self
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }

    func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
        assetURL as NSURL
    }
}

private struct CaptureDraft: Encodable {
    let schemaVersion = "roomplan-capture/v1"
    let draftID: String
    let createdAt: String
    let source: CaptureSource
    let coordinateSystem = CoordinateSystem()
    let room: CapturedRoomPayload
    let assets: [CaptureAsset]
    let provenance: CaptureProvenance

    enum CodingKeys: String, CodingKey {
        case source, room, assets, provenance
        case schemaVersion = "schema_version"
        case draftID = "draft_id"
        case createdAt = "created_at"
        case coordinateSystem = "coordinate_system"
    }
}

private struct LiveCaptureUpdate: Encodable {
    let draftID: String
    let state: String
    let room: CapturedRoomPayload
    let guidance: String
    let addedCount: Int
    let changedCount: Int
    let removedCount: Int
    let updatedAt: String

    enum CodingKeys: String, CodingKey {
        case state, room, guidance
        case draftID = "draft_id"
        case addedCount = "added_count"
        case changedCount = "changed_count"
        case removedCount = "removed_count"
        case updatedAt = "updated_at"
    }
}

private final class RoomCaptureController: UIViewController, RoomCaptureSessionDelegate {
    private var captureView: RoomCaptureView?
    private var builder: RoomBuilder?
    private var finishButton: UIButton?
    private var liveCountsLabel: UILabel?
    private var liveChangeLabel: UILabel?
    private var liveInstructionLabel: UILabel?
    private let draftID = UUID().uuidString.lowercased()
    private let startedAt = Date()
    private var latestRoom: CapturedRoom?
    private var liveGuidance = "Move slowly around the room to build coverage."
    private var lastLivePublishAt = Date.distantPast
    private let livePublishInterval: TimeInterval = 0.5
    private var finished = false
    private var building = false
    private var addedCount = 0
    private var changedCount = 0
    private var removedCount = 0

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Capture a room"
        view.backgroundColor = .systemBackground

        let capture = RoomCaptureView(frame: view.bounds)
        capture.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        capture.captureSession.delegate = self
        view.addSubview(capture)
        captureView = capture
        builder = RoomBuilder(options: [.beautifyObjects])

        let livePanel = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
        livePanel.layer.cornerRadius = 16
        livePanel.clipsToBounds = true
        livePanel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(livePanel)

        let title = UILabel()
        title.text = "Live room scan"
        title.font = .preferredFont(forTextStyle: .headline)
        title.textColor = .label

        let counts = UILabel()
        counts.text = "Waiting for room details…"
        counts.font = .monospacedDigitSystemFont(ofSize: 13, weight: .medium)
        counts.textColor = .secondaryLabel
        counts.numberOfLines = 2
        liveCountsLabel = counts

        let changes = UILabel()
        changes.text = "No scan changes yet"
        changes.font = .preferredFont(forTextStyle: .caption1)
        changes.textColor = .secondaryLabel
        liveChangeLabel = changes

        let instruction = UILabel()
        instruction.text = "Move slowly around the room to build coverage."
        instruction.font = .preferredFont(forTextStyle: .subheadline)
        instruction.textColor = .label
        instruction.numberOfLines = 2
        liveInstructionLabel = instruction

        let stack = UIStackView(arrangedSubviews: [title, counts, changes, instruction])
        stack.axis = .vertical
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        livePanel.contentView.addSubview(stack)

        NSLayoutConstraint.activate([
            livePanel.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            livePanel.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            livePanel.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            stack.leadingAnchor.constraint(equalTo: livePanel.contentView.leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: livePanel.contentView.trailingAnchor, constant: -14),
            stack.topAnchor.constraint(equalTo: livePanel.contentView.topAnchor, constant: 12),
            stack.bottomAnchor.constraint(equalTo: livePanel.contentView.bottomAnchor, constant: -12),
        ])

        let finish = UIButton(type: .system)
        finish.setTitle("Finish and save", for: .normal)
        finish.titleLabel?.font = .preferredFont(forTextStyle: .headline)
        finish.backgroundColor = .systemBackground
        finish.layer.cornerRadius = 12
        finish.translatesAutoresizingMaskIntoConstraints = false
        finish.addTarget(self, action: #selector(stopCapture), for: .touchUpInside)
        view.addSubview(finish)
        finishButton = finish

        let cancel = UIBarButtonItem(title: "Cancel", style: .plain, target: self, action: #selector(cancelCapture))
        navigationItem.leftBarButtonItem = cancel
        NSLayoutConstraint.activate([
            finish.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            finish.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
            finish.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -18),
            finish.heightAnchor.constraint(equalToConstant: 52),
        ])
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        captureView?.captureSession.run(configuration: RoomCaptureSession.Configuration())
        UIApplication.shared.isIdleTimerDisabled = true
        publishLiveUpdate(state: "scanning", force: true)
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if !finished { captureView?.captureSession.stop() }
        UIApplication.shared.isIdleTimerDisabled = false
    }

    @objc private func stopCapture() {
        guard !building else { return }
        building = true
        finishButton?.isEnabled = false
        publishLiveUpdate(state: "building", force: true)
        captureView?.captureSession.stop()
        title = "Building room model…"
    }

    @objc private func cancelCapture() {
        finished = true
        publishLiveUpdate(state: "cancelled", force: true)
        captureView?.captureSession.stop()
        dismiss(animated: true)
    }

    func captureSession(_ session: RoomCaptureSession, didEndWith data: CapturedRoomData, error: Error?) {
        guard !finished else { return }
        if let error {
            complete(error: error.localizedDescription)
            return
        }
        guard let builder else {
            complete(error: "RoomPlan could not initialize its model builder.")
            return
        }
        Task {
            do {
                let room = try await builder.capturedRoom(from: data)
                guard !finished else { return }
                latestRoom = room
                let summary = try save(room: room)
                complete(summary: summary)
            } catch {
                if !finished {
                    complete(error: "RoomPlan could not save this scan: \(error.localizedDescription)")
                }
            }
        }
    }

    func captureSession(_ session: RoomCaptureSession, didUpdate room: CapturedRoom) {
        DispatchQueue.main.async {
            guard !self.finished else { return }
            self.latestRoom = room
            self.liveCountsLabel?.text = self.liveCounts(for: room)
            self.publishLiveUpdate(state: self.building ? "building" : "scanning", force: false)
        }
    }

    func captureSession(_ session: RoomCaptureSession, didAdd room: CapturedRoom) {
        recordLiveChange("New scan detail", count: self.elementCount(in: room), kind: .added)
    }

    func captureSession(_ session: RoomCaptureSession, didChange room: CapturedRoom) {
        recordLiveChange("Refined scan detail", count: self.elementCount(in: room), kind: .changed)
    }

    func captureSession(_ session: RoomCaptureSession, didRemove room: CapturedRoom) {
        recordLiveChange("Rechecked uncertain detail", count: self.elementCount(in: room), kind: .removed)
    }

    func captureSession(_ session: RoomCaptureSession, didProvide instruction: RoomCaptureSession.Instruction) {
        DispatchQueue.main.async {
            guard !self.finished else { return }
            self.liveGuidance = self.guidance(for: instruction)
            self.liveInstructionLabel?.text = self.liveGuidance
            self.publishLiveUpdate(state: self.building ? "building" : "scanning", force: false)
        }
    }

    private enum LiveChangeKind {
        case added
        case changed
        case removed
    }

    private func elementCount(in room: CapturedRoom) -> Int {
        room.walls.count + room.doors.count + room.windows.count + room.openings.count
            + room.floors.count + room.objects.count
    }

    private func liveCounts(for room: CapturedRoom) -> String {
        let surfaces = room.walls.count + room.floors.count
        let openings = room.doors.count + room.windows.count + room.openings.count
        return "\(surfaces) walls and floors · \(openings) openings · \(room.objects.count) objects"
    }

    private func recordLiveChange(_ label: String, count: Int, kind: LiveChangeKind) {
        DispatchQueue.main.async {
            guard !self.finished else { return }
            switch kind {
            case .added: self.addedCount += count
            case .changed: self.changedCount += count
            case .removed: self.removedCount += count
            }
            self.liveChangeLabel?.text = "\(label) · +\(self.addedCount) · refined \(self.changedCount) · rechecked \(self.removedCount)"
            self.publishLiveUpdate(state: self.building ? "building" : "scanning", force: false)
        }
    }

    private func guidance(for instruction: RoomCaptureSession.Instruction) -> String {
        switch instruction {
        case .normal:
            return "Keep moving slowly to improve room coverage."
        case .moveCloseToWall:
            return "Move closer to the wall."
        case .moveAwayFromWall:
            return "Move a little farther from the wall."
        case .turnOnLight:
            return "Add light so RoomPlan can see the room clearly."
        case .slowDown:
            return "Move the phone more slowly."
        case .lowTexture:
            return "Scan slowly across surfaces with more visible detail."
        @unknown default:
            return "Follow RoomPlan’s camera guidance."
        }
    }

    private func publishLiveUpdate(state: String, force: Bool) {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { self.publishLiveUpdate(state: state, force: force) }
            return
        }
        let now = Date()
        guard force || now.timeIntervalSince(lastLivePublishAt) >= livePublishInterval else { return }
        lastLivePublishAt = now

        let payload = LiveCaptureUpdate(
            draftID: draftID,
            state: state,
            room: latestRoom.map { map($0) } ?? CapturedRoomPayload(id: "", surfaces: [], openings: [], objects: []),
            guidance: liveGuidance,
            addedCount: addedCount,
            changedCount: changedCount,
            removedCount: removedCount,
            updatedAt: ISO8601DateFormatter().string(from: now)
        )
        guard let json = try? JSONEncoder().encode(payload),
              let text = String(data: json, encoding: .utf8) else { return }
        text.withCString { sjel_ios_roomplan_live($0) }
    }

    private func save(room: CapturedRoom) throws -> String {
        let finishedAt = Date()
        let root = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        ).appendingPathComponent("RoomPlan", isDirectory: true)
        let directory = root.appendingPathComponent(draftID, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let assetURL = directory.appendingPathComponent("parametric.usdz")
        try room.export(to: assetURL, exportOptions: .parametric)
        let assetData = try Data(contentsOf: assetURL)
        let digest = SHA256.hash(data: assetData).map { String(format: "%02x", $0) }.joined()
        let asset = CaptureAsset(
            assetID: "\(draftID)-parametric", byteLength: UInt64(assetData.count),
            sha256: digest, storageToken: "\(draftID)/parametric.usdz"
        )
        let mappedRoom = map(room)
        let surfaces = mappedRoom.surfaces
        let draft = CaptureDraft(
            draftID: draftID,
            createdAt: ISO8601DateFormatter().string(from: finishedAt),
            source: CaptureSource(
                roomplanVersion: ProcessInfo.processInfo.operatingSystemVersionString,
                appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ),
            room: mappedRoom,
            assets: [asset],
            provenance: CaptureProvenance(
                captureStartedAt: ISO8601DateFormatter().string(from: startedAt),
                captureFinishedAt: ISO8601DateFormatter().string(from: finishedAt)
            )
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let json = try encoder.encode(draft)
        try json.write(to: directory.appendingPathComponent("draft.json"), options: [.atomic])
        try UploadReceipt(state: "pending", uploadedAt: nil).write(to: directory)
        if let evidence = try? JSONEncoder().encode(room) {
            try? evidence.write(to: directory.appendingPathComponent("captured-room.json"), options: [.atomic])
        }
        return "Saved offline · \(surfaces.count) surfaces · \(room.openings.count) openings · \(room.objects.count) objects"
    }

    private func map(_ room: CapturedRoom) -> CapturedRoomPayload {
        CapturedRoomPayload(
            id: room.identifier.uuidString.lowercased(),
            surfaces: room.walls.map { map($0) } + room.doors.map { map($0) }
                + room.windows.map { map($0) } + room.floors.map { map($0) },
            openings: room.openings.map { map($0) },
            objects: room.objects.map { map($0) }
        )
    }

    private func map(_ surface: CapturedRoom.Surface) -> RoomElement {
        RoomElement(
            id: "surface-\(surface.identifier.uuidString.lowercased())",
            category: String(describing: surface.category),
            dimensionsM: [surface.dimensions.x, surface.dimensions.y, surface.dimensions.z],
            transform: matrix(surface.transform), confidence: confidence(surface.confidence),
            sourceID: surface.identifier.uuidString.lowercased()
        )
    }

    private func map(_ object: CapturedRoom.Object) -> RoomElement {
        RoomElement(
            id: "object-\(object.identifier.uuidString.lowercased())",
            category: String(describing: object.category),
            dimensionsM: [object.dimensions.x, object.dimensions.y, object.dimensions.z],
            transform: matrix(object.transform), confidence: confidence(object.confidence),
            sourceID: object.identifier.uuidString.lowercased()
        )
    }

    private func matrix(_ value: simd_float4x4) -> [Float] {
        [value.columns.0.x, value.columns.0.y, value.columns.0.z, value.columns.0.w,
         value.columns.1.x, value.columns.1.y, value.columns.1.z, value.columns.1.w,
         value.columns.2.x, value.columns.2.y, value.columns.2.z, value.columns.2.w,
         value.columns.3.x, value.columns.3.y, value.columns.3.z, value.columns.3.w]
    }

    private func confidence(_ value: CapturedRoom.Confidence) -> String {
        switch value {
        case .high: return "high"
        case .medium: return "medium"
        case .low: return "low"
        @unknown default: return "low"
        }
    }

    private func complete(summary: String? = nil, error: String? = nil) {
        DispatchQueue.main.async {
            self.publishLiveUpdate(state: error == nil ? "saved" : "failed", force: true)
            self.finished = true
            UIApplication.shared.isIdleTimerDisabled = false
            if let error {
                error.withCString { sjel_ios_roomplan_result($0) }
            } else {
                (summary ?? "Room capture saved.").withCString { sjel_ios_roomplan_result($0) }
                sjelIosLoadRoomplanArchive()
                sjelIosLoadRoomplanUpload()
            }
            self.dismiss(animated: true)
        }
    }
}

private extension UploadReceipt {
    func write(to directory: URL) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        try encoder.encode(self).write(
            to: directory.appendingPathComponent("upload-state.json"), options: [.atomic]
        )
    }
}

private func roomPlanRoot() throws -> URL {
    try FileManager.default.url(
        for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
    ).appendingPathComponent("RoomPlan", isDirectory: true)
}

private func roomPlanEntries(at root: URL) -> [(URL, [String: Any], UploadReceipt)] {
    let legacyPointer = root.appendingPathComponent("current.json")
    if let data = try? Data(contentsOf: legacyPointer),
       let draft = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
       let draftID = draft["draft_id"] as? String,
       !draftID.isEmpty,
       draftID.utf8.allSatisfy({
           ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122) || ($0 >= 48 && $0 <= 57) || $0 == 45 || $0 == 95
       }) {
        let directory = root.appendingPathComponent(draftID, isDirectory: true)
        let draftURL = directory.appendingPathComponent("draft.json")
        if FileManager.default.fileExists(atPath: directory.path) {
            if !FileManager.default.fileExists(atPath: draftURL.path) {
                try? data.write(to: draftURL, options: [.atomic])
            }
            let receiptURL = directory.appendingPathComponent("upload-state.json")
            if !FileManager.default.fileExists(atPath: receiptURL.path) {
                try? UploadReceipt(state: "pending", uploadedAt: nil).write(to: directory)
            }
        }
    }
    guard let urls = try? FileManager.default.contentsOfDirectory(
        at: root, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles]
    ) else { return [] }
    return urls.compactMap { directory in
        guard (try? directory.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true,
              let draftData = try? Data(contentsOf: directory.appendingPathComponent("draft.json")),
              let draft = try? JSONSerialization.jsonObject(with: draftData) as? [String: Any],
              FileManager.default.fileExists(
                  atPath: directory.appendingPathComponent("parametric.usdz").path
              ) else {
            return nil
        }
        let receiptURL = directory.appendingPathComponent("upload-state.json")
        let receipt = (try? Data(contentsOf: receiptURL))
            .flatMap { try? JSONDecoder().decode(UploadReceipt.self, from: $0) }
            ?? UploadReceipt(state: "pending", uploadedAt: nil)
        if !FileManager.default.fileExists(atPath: receiptURL.path) {
            try? receipt.write(to: directory)
        }
        return (directory, draft, receipt)
    }.sorted {
        let left = ($0.1["created_at"] as? String) ?? ""
        let right = ($1.1["created_at"] as? String) ?? ""
        return left < right
    }
}

private enum LegacyRoomPlanImportError: LocalizedError {
    case chooseLocalFolder
    case archiveNotReady
    case cloudFolder
    case noCaptures
    case invalidCapture(String)
    case conflictingCapture(String)

    var errorDescription: String? {
        switch self {
        case .chooseLocalFolder:
            return "Choose the RoomPlan-GPUI-Migration folder under On My iPhone > Sjel."
        case .archiveNotReady:
            return "The local scan copy is still being prepared. Open the old Sjel app and try again shortly."
        case .cloudFolder:
            return "Choose the legacy archive under On My iPhone; cloud locations are not imported."
        case .noCaptures:
            return "No scans were found in this local archive. The old app may not have any captures yet."
        case .invalidCapture(let id):
            return "Legacy scan \(id) is incomplete or failed its integrity check."
        case .conflictingCapture(let id):
            return "Scan \(id) already exists here with different data; no file was replaced."
        }
    }
}

private struct LegacyRoomPlanImportEntry {
    let draftID: String
    let directory: URL
    let draftData: Data
    let assetURL: URL
}

private struct LegacyRoomPlanImportReport {
    let imported: Int
    let existing: Int
    let failed: Int
}

private func regularLocalFile(_ url: URL) -> Bool {
    guard let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey]) else {
        return false
    }
    return values.isRegularFile == true && values.isSymbolicLink != true
}

private func sameRoomPlanDraft(_ lhs: Data, _ rhs: Data) -> Bool {
    guard let left = try? JSONSerialization.jsonObject(with: lhs) as? [String: Any],
          let right = try? JSONSerialization.jsonObject(with: rhs) as? [String: Any] else {
        return false
    }
    return (left as NSDictionary).isEqual(right as NSDictionary)
}

private func legacyRoomPlanImportEntries(from root: URL) throws -> [LegacyRoomPlanImportEntry] {
    if (try? root.resourceValues(forKeys: [.isUbiquitousItemKey]).isUbiquitousItem) == true {
        throw LegacyRoomPlanImportError.cloudFolder
    }
    let fileManager = FileManager.default
    let directDraft = root.appendingPathComponent("draft.json")
    let directories: [URL]
    if fileManager.fileExists(atPath: directDraft.path) {
        directories = [root]
    } else {
        guard fileManager.fileExists(atPath: root.appendingPathComponent(".ready").path) else {
            throw root.lastPathComponent == "RoomPlan-GPUI-Migration"
                ? LegacyRoomPlanImportError.archiveNotReady
                : LegacyRoomPlanImportError.chooseLocalFolder
        }
        directories = try fileManager.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey],
            options: [.skipsHiddenFiles]
        ).filter { url in
            let values = try? url.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
            return values?.isDirectory == true && values?.isSymbolicLink != true
        }
    }

    var entries = [LegacyRoomPlanImportEntry]()
    for directory in directories {
        let draftURL = directory.appendingPathComponent("draft.json")
        let assetURL = directory.appendingPathComponent("parametric.usdz")
        guard fileManager.fileExists(atPath: draftURL.path) else { continue }
        guard regularLocalFile(draftURL), regularLocalFile(assetURL),
              let draftData = try? Data(contentsOf: draftURL),
              let draft = try? JSONSerialization.jsonObject(with: draftData) as? [String: Any],
              draft["schema_version"] as? String == "roomplan-capture/v1",
              let draftID = draft["draft_id"] as? String,
              draftID == directory.lastPathComponent,
              draftID.utf8.allSatisfy({
                  ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122) || ($0 >= 48 && $0 <= 57) || $0 == 45 || $0 == 95
              }),
              let assets = draft["assets"] as? [[String: Any]],
              let parametric = assets.first(where: { $0["role"] as? String == "parametric" }),
              let expectedHash = parametric["sha256"] as? String,
              let expectedLength = parametric["byte_length"] as? NSNumber else {
            throw LegacyRoomPlanImportError.invalidCapture(directory.lastPathComponent)
        }
        let assetData = try Data(contentsOf: assetURL, options: [.mappedIfSafe])
        let actualHash = SHA256.hash(data: assetData).map { String(format: "%02x", $0) }.joined()
        guard expectedLength.uint64Value == UInt64(assetData.count),
              actualHash.caseInsensitiveCompare(expectedHash) == .orderedSame else {
            throw LegacyRoomPlanImportError.invalidCapture(draftID)
        }
        entries.append(
            LegacyRoomPlanImportEntry(
                draftID: draftID,
                directory: directory,
                draftData: draftData,
                assetURL: assetURL
            )
        )
    }
    guard !entries.isEmpty else { throw LegacyRoomPlanImportError.noCaptures }
    return entries
}

private func importLegacyRoomPlanArchive(from source: URL) throws -> LegacyRoomPlanImportReport {
    let entries = try legacyRoomPlanImportEntries(from: source)
    let fileManager = FileManager.default
    let root = try roomPlanRoot()
    try fileManager.createDirectory(at: root, withIntermediateDirectories: true)
    var imported = 0
    var existing = 0
    var failed = 0

    for entry in entries {
        let destination = root.appendingPathComponent(entry.draftID, isDirectory: true)
        guard fileManager.fileExists(atPath: destination.path) else { continue }
        let values = try destination.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        let storedDraftURL = destination.appendingPathComponent("draft.json")
        let storedAssetURL = destination.appendingPathComponent("parametric.usdz")
        guard values.isDirectory == true, values.isSymbolicLink != true,
              regularLocalFile(storedDraftURL), regularLocalFile(storedAssetURL) else {
            throw LegacyRoomPlanImportError.conflictingCapture(entry.draftID)
        }
        let storedDraft = try Data(contentsOf: storedDraftURL)
        let storedAsset = try Data(contentsOf: storedAssetURL, options: [.mappedIfSafe])
        let incomingAsset = try Data(contentsOf: entry.assetURL, options: [.mappedIfSafe])
        guard sameRoomPlanDraft(storedDraft, entry.draftData), storedAsset == incomingAsset else {
            throw LegacyRoomPlanImportError.conflictingCapture(entry.draftID)
        }
        existing += 1
    }

    for entry in entries {
        let destination = root.appendingPathComponent(entry.draftID, isDirectory: true)
        if fileManager.fileExists(atPath: destination.path) {
            continue
        }

        let staging = root.appendingPathComponent(
            ".legacy-import-\(UUID().uuidString)",
            isDirectory: true
        )
        do {
            try fileManager.createDirectory(at: staging, withIntermediateDirectories: false)
            try entry.draftData.write(to: staging.appendingPathComponent("draft.json"), options: [.atomic])
            try fileManager.copyItem(
                at: entry.assetURL,
                to: staging.appendingPathComponent("parametric.usdz")
            )
            for name in ["mesh.usdz", "captured-room.json"] {
                let sourceFile = entry.directory.appendingPathComponent(name)
                if regularLocalFile(sourceFile) {
                    try fileManager.copyItem(at: sourceFile, to: staging.appendingPathComponent(name))
                }
            }
            try UploadReceipt(state: "pending", uploadedAt: nil).write(to: staging)
            try fileManager.moveItem(at: staging, to: destination)
            imported += 1
        } catch {
            try? fileManager.removeItem(at: staging)
            failed += 1
            NSLog("RoomPlan: legacy scan \(entry.draftID) was not imported: \(error.localizedDescription)")
        }
    }
    return LegacyRoomPlanImportReport(imported: imported, existing: existing, failed: failed)
}

private final class LegacyRoomPlanImportCoordinator: NSObject, UIDocumentPickerDelegate {
    static let shared = LegacyRoomPlanImportCoordinator()

    func present() {
        guard let root = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .compactMap({ $0.keyWindow ?? $0.windows.first })
            .first?.rootViewController else { return }
        var top = root
        while let presented = top.presentedViewController { top = presented }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder], asCopy: false)
        picker.allowsMultipleSelection = false
        picker.delegate = self
        top.present(picker, animated: true)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let source = urls.first else { return }
        let accessed = source.startAccessingSecurityScopedResource()
        DispatchQueue.global(qos: .userInitiated).async {
            defer { if accessed { source.stopAccessingSecurityScopedResource() } }
            do {
                let report = try importLegacyRoomPlanArchive(from: source)
                DispatchQueue.main.async {
                    let message = "Imported \(report.imported) legacy scans; \(report.existing) were already present; \(report.failed) could not be copied."
                    message.withCString {
                        sjel_ios_roomplan_result($0)
                    }
                    sjelIosLoadRoomplanArchive()
                    sjelIosLoadRoomplanStatus()
                }
            } catch {
                DispatchQueue.main.async {
                    error.localizedDescription.withCString { sjel_ios_roomplan_result($0) }
                    sjelIosLoadRoomplanArchive()
                    sjelIosLoadRoomplanStatus()
                }
            }
        }
    }
}

@_cdecl("sjel_ios_import_legacy_roomplan")
public func sjelIosImportLegacyRoomplan() {
    DispatchQueue.main.async {
        LegacyRoomPlanImportCoordinator.shared.present()
    }
}

@_cdecl("sjel_ios_preview_roomplan")
public func sjelIosPreviewRoomplan(_ text: UnsafePointer<CChar>) {
    let draftID = String(cString: text)
    guard !draftID.isEmpty,
          draftID.utf8.allSatisfy({
              ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122) || ($0 >= 48 && $0 <= 57) || $0 == 45 || $0 == 95
          }) else { return }
    DispatchQueue.main.async {
        do {
            let root = try roomPlanRoot()
            let entries = roomPlanEntries(at: root)
            guard entries.contains(where: {
                $0.0.lastPathComponent == draftID && ($0.1["draft_id"] as? String) == draftID
            }) else {
                "This scan is not available in the phone archive.".withCString { sjel_ios_roomplan_result($0) }
                return
            }
            let assetURL = root.appendingPathComponent(draftID, isDirectory: true)
                .appendingPathComponent("parametric.usdz")
            guard FileManager.default.isReadableFile(atPath: assetURL.path) else {
                "The 3D scan file is unavailable.".withCString { sjel_ios_roomplan_result($0) }
                return
            }
            presentRoomPlanPreview(at: assetURL)
        } catch {
            "The saved 3D scan could not be opened.".withCString { sjel_ios_roomplan_result($0) }
        }
    }
}

@_cdecl("sjel_ios_cache_preview_roomplan")
public func sjelIosCachePreviewRoomplan(
    _ idText: UnsafePointer<CChar>,
    _ draftText: UnsafePointer<CChar>,
    _ bytes: UnsafePointer<UInt8>,
    _ length: UInt
) {
    let draftID = String(cString: idText)
    let draftJSON = String(cString: draftText)
    guard !draftID.isEmpty,
          draftID.utf8.allSatisfy({
              ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122) || ($0 >= 48 && $0 <= 57) || $0 == 45 || $0 == 95
          }),
          length > 0,
          length <= UInt(Int.max) else { return }
    let assetData = Data(bytes: bytes, count: Int(length))
    DispatchQueue.main.async {
        do {
            let assetURL = try cacheRoomPlanRevision(
                draftID: draftID,
                draftJSON: draftJSON,
                assetData: assetData
            )
            sjelIosLoadRoomplanArchive()
            presentRoomPlanPreview(at: assetURL)
        } catch {
            "The 3D scan could not be cached on this phone.".withCString { sjel_ios_roomplan_result($0) }
        }
    }
}

private func cacheRoomPlanRevision(draftID: String, draftJSON: String, assetData: Data) throws -> URL {
    let root = try roomPlanRoot()
    let directory = root.appendingPathComponent(draftID, isDirectory: true)
    let fileManager = FileManager.default
    let incomingDraftData = Data(draftJSON.utf8)
    guard let incomingDraft = try JSONSerialization.jsonObject(with: incomingDraftData) as? [String: Any],
          incomingDraft["draft_id"] as? String == draftID else {
        throw CocoaError(.fileReadCorruptFile)
    }
    guard let parametric = (incomingDraft["assets"] as? [[String: Any]])?.first(where: {
              ($0["role"] as? String) == "parametric"
          }),
          let expectedHash = parametric["sha256"] as? String,
          let expectedLength = parametric["byte_length"] as? NSNumber,
          expectedLength.uint64Value == UInt64(assetData.count) else {
        throw CocoaError(.fileReadCorruptFile)
    }
    let actualHash = SHA256.hash(data: assetData).map { String(format: "%02x", $0) }.joined()
    guard actualHash.caseInsensitiveCompare(expectedHash) == .orderedSame else {
        throw CocoaError(.fileReadCorruptFile)
    }

    let assetURL = directory.appendingPathComponent("parametric.usdz")
    if fileManager.fileExists(atPath: directory.path) {
        let values = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        let draftURL = directory.appendingPathComponent("draft.json")
        guard values.isDirectory == true, values.isSymbolicLink != true,
              regularLocalFile(draftURL), regularLocalFile(assetURL),
              let storedDraftData = try? Data(contentsOf: draftURL),
              sameRoomPlanDraft(storedDraftData, incomingDraftData),
              (try? Data(contentsOf: assetURL)) == assetData else {
            throw CocoaError(.fileWriteFileExists)
        }
        return assetURL
    }

    try fileManager.createDirectory(at: root, withIntermediateDirectories: true)
    let staging = root.appendingPathComponent(".roomplan-cache-\(UUID().uuidString)", isDirectory: true)
    do {
        try fileManager.createDirectory(at: staging, withIntermediateDirectories: false)
        try incomingDraftData.write(to: staging.appendingPathComponent("draft.json"), options: [.atomic])
        try assetData.write(to: staging.appendingPathComponent("parametric.usdz"), options: [.atomic])
        try UploadReceipt(state: "cached", uploadedAt: nil).write(to: staging)
        try fileManager.moveItem(at: staging, to: directory)
    } catch {
        try? fileManager.removeItem(at: staging)
        throw error
    }
    return assetURL
}

private func presentRoomPlanPreview(at assetURL: URL) {
    guard let rootController = UIApplication.shared.connectedScenes
        .compactMap({ $0 as? UIWindowScene })
        .compactMap({ $0.keyWindow ?? $0.windows.first })
        .first?.rootViewController else { return }
    var top = rootController
    while let presented = top.presentedViewController { top = presented }
    top.present(RoomPlanPreviewController(assetURL: assetURL), animated: true)
}

@_cdecl("sjel_ios_load_roomplan_status")
public func sjelIosLoadRoomplanStatus() {
    DispatchQueue.main.async {
        do {
            let entries = roomPlanEntries(at: try roomPlanRoot())
            guard let (_, draft, _) = entries.last else {
                "No room capture saved yet.".withCString { sjel_ios_roomplan_result($0) }
                return
            }
            let room = draft["room"] as? [String: Any]
            let surfaces = (room?["surfaces"] as? [Any])?.count ?? 0
            let openings = (room?["openings"] as? [Any])?.count ?? 0
            let objects = (room?["objects"] as? [Any])?.count ?? 0
            let pending = entries.filter { $0.2.state == "pending" }.count
            "\(entries.count) saved scans · \(pending) waiting to sync · latest: \(surfaces) surfaces, \(openings) openings, \(objects) objects".withCString {
                sjel_ios_roomplan_result($0)
            }
        } catch {
            "No room capture saved yet.".withCString { sjel_ios_roomplan_result($0) }
        }
    }
}

@_cdecl("sjel_ios_load_roomplan_upload")
public func sjelIosLoadRoomplanUpload() {
    DispatchQueue.global(qos: .utility).async {
        do {
            let entries = roomPlanEntries(at: try roomPlanRoot())
            guard let (_, draftObject, _) = entries.first(where: { $0.2.state == "pending" }),
                  let draftID = draftObject["draft_id"] as? String, !draftID.isEmpty else {
                "No pending room scans to sync.".withCString { sjel_ios_roomplan_result($0) }
                return
            }
            let assetURL = try roomPlanRoot().appendingPathComponent(draftID, isDirectory: true)
                .appendingPathComponent("parametric.usdz")
            let asset = try Data(contentsOf: assetURL)
            let body: [String: Any] = [
                "draft": draftObject,
                "usdz_base64": asset.base64EncodedString(),
            ]
            let encoded = try JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])
            guard let text = String(data: encoded, encoding: .utf8) else {
                throw CocoaError(.fileReadInapplicableStringEncoding)
            }
            text.withCString { sjel_ios_roomplan_upload($0) }
        } catch {
            "No saved room scan is available to send.".withCString { sjel_ios_roomplan_result($0) }
        }
    }
}

@_cdecl("sjel_ios_load_roomplan_archive")
public func sjelIosLoadRoomplanArchive() {
    DispatchQueue.global(qos: .utility).async {
        do {
            let entries = roomPlanEntries(at: try roomPlanRoot())
            let records = entries.map { _, draft, receipt in
                ["draft": draft, "sync_state": receipt.state] as [String: Any]
            }
            let data = try JSONSerialization.data(withJSONObject: records, options: [.sortedKeys])
            guard let text = String(data: data, encoding: .utf8) else {
                throw CocoaError(.fileReadInapplicableStringEncoding)
            }
            text.withCString { sjel_ios_roomplan_archive($0) }
        } catch {
            "[]".withCString { sjel_ios_roomplan_archive($0) }
        }
    }
}

@_cdecl("sjel_ios_mark_roomplan_uploaded")
public func sjelIosMarkRoomplanUploaded(_ text: UnsafePointer<CChar>) {
    let draftID = String(cString: text)
    DispatchQueue.main.async {
        do {
            let directory = try roomPlanRoot().appendingPathComponent(draftID, isDirectory: true)
            let draftData = try Data(contentsOf: directory.appendingPathComponent("draft.json"))
            guard let draft = try JSONSerialization.jsonObject(with: draftData) as? [String: Any],
                  draft["draft_id"] as? String == draftID else { return }
            try UploadReceipt(state: "synced", uploadedAt: ISO8601DateFormatter().string(from: Date()))
                .write(to: directory)
            "Room scan synced · \(draftID)".withCString { sjel_ios_roomplan_result($0) }
            sjelIosLoadRoomplanArchive()
            sjelIosLoadRoomplanUpload()
        } catch {
            "Scan reached the Mac, but its local sync receipt could not be saved.".withCString {
                sjel_ios_roomplan_result($0)
            }
        }
    }
}

@_cdecl("sjel_ios_present_roomplan")
public func sjelIosPresentRoomplan() {
    DispatchQueue.main.async {
        guard RoomCaptureSession.isSupported else {
            "RoomPlan requires a LiDAR-equipped iPhone or iPad.".withCString { sjel_ios_roomplan_result($0) }
            return
        }
        let present: () -> Void = {
            guard let root = UIApplication.shared.connectedScenes
                .compactMap({ $0 as? UIWindowScene })
                .compactMap({ $0.keyWindow ?? $0.windows.first })
                .first?.rootViewController else { return }
            var top = root
            while let presented = top.presentedViewController { top = presented }
            top.present(UINavigationController(rootViewController: RoomCaptureController()), animated: true)
        }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            present()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { granted in
                DispatchQueue.main.async {
                    if granted { present() }
                    else { "Camera permission is required for room capture.".withCString { sjel_ios_roomplan_result($0) } }
                }
            }
        default:
            "Allow camera access in iOS Settings to capture a room.".withCString { sjel_ios_roomplan_result($0) }
        }
    }
}
