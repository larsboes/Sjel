import AVFoundation
import CryptoKit
import Foundation
import QuickLook
import RoomPlan
import Tauri
import UIKit
import WebKit

private struct StartCaptureArguments: Decodable {
  let includeMesh: Bool?
  let mode: String?
}

private struct LegacyImportManifest: Decodable {
  let roomId: String
  let expectedSha256: String

  enum CodingKeys: String, CodingKey {
    case roomId = "room_id"
    case expectedSha256 = "expected_sha256"
  }
}

private struct CaptureSource: Codable {
  let platform = "ios"
  let roomplanVersion: String?
  let appVersion: String?

  enum CodingKeys: String, CodingKey {
    case platform
    case roomplanVersion = "roomplan_version"
    case appVersion = "app_version"
  }
}

private struct CoordinateSystem: Codable {
  let units = "meters"
  let upAxis = "Y"
  let handedness: String? = nil

  enum CodingKeys: String, CodingKey {
    case units
    case upAxis = "up_axis"
    case handedness
  }
}

private struct RoomElement: Codable {
  let id: String
  let category: String
  let dimensionsM: [Float]
  let transform: [Float]
  let confidence: String
  let sourceID: String

  enum CodingKeys: String, CodingKey {
    case id
    case category
    case dimensionsM = "dimensions_m"
    case transform
    case confidence
    case sourceID = "source_id"
  }
}

private struct CapturedRoomPayload: Codable {
  let id: String
  let surfaces: [RoomElement]
  let openings: [RoomElement]
  let objects: [RoomElement]
}

private struct CaptureAsset: Codable {
  let assetID: String
  let role: String
  let format = "usdz"
  let byteLength: UInt64
  let sha256: String
  let storageToken: String

  enum CodingKeys: String, CodingKey {
    case assetID = "asset_id"
    case role
    case format
    case byteLength = "byte_length"
    case sha256
    case storageToken = "storage_token"
  }
}

private struct CaptureProvenance: Codable {
  let captureMode: String?
  let parentRevisionID: String?
  let confidence: String? = nil
  let captureStartedAt: String?
  let captureFinishedAt: String?

  enum CodingKeys: String, CodingKey {
    case captureMode = "capture_mode"
    case parentRevisionID = "parent_revision_id"
    case confidence
    case captureStartedAt = "capture_started_at"
    case captureFinishedAt = "capture_finished_at"
  }
}

private struct CaptureDraftPayload: Codable {
  let schemaVersion = "roomplan-capture/v1"
  let draftID: String
  let createdAt: String
  let source: CaptureSource
  let coordinateSystem = CoordinateSystem()
  let room: CapturedRoomPayload
  let assets: [CaptureAsset]
  let provenance: CaptureProvenance

  enum CodingKeys: String, CodingKey {
    case schemaVersion = "schema_version"
    case draftID = "draft_id"
    case createdAt = "created_at"
    case source
    case coordinateSystem = "coordinate_system"
    case room
    case assets
    case provenance
  }
}

private struct CaptureCompletedEvent: Codable {
  let draft: CaptureDraftPayload
}

private struct CaptureProgressEvent: Codable {
  let phase: String
  let draftID: String?

  enum CodingKeys: String, CodingKey {
    case phase
    case draftID = "draft_id"
  }
}

private struct LegacyObservation: Codable {
  let format: String
  let byteLength: UInt64
  let sha256: String
  let metersPerUnit: Double
  let upAxis: String
  let exportObservation: LegacyExportObservation
  let status: String
  let sourceContract: String

  enum CodingKeys: String, CodingKey {
    case format
    case byteLength = "byte_length"
    case sha256
    case metersPerUnit = "meters_per_unit"
    case upAxis = "up_axis"
    case exportObservation = "export_observation"
    case status
    case sourceContract = "source_contract"
  }
}

private struct LegacyExportObservation: Codable {
  let roomGroups: Int
  let meshAssets: Int
  let categoryCounts: [String: Int]

  enum CodingKeys: String, CodingKey {
    case roomGroups = "room_groups"
    case meshAssets = "mesh_assets"
    case categoryCounts = "category_counts"
  }
}

private struct LegacyReferenceAsset: Codable {
  let format: String
  let byteLength: UInt64
  let sha256: String
  let storageToken: String

  enum CodingKeys: String, CodingKey {
    case format
    case byteLength = "byte_length"
    case sha256
    case storageToken = "storage_token"
  }
}

private struct LegacyReferencePayload: Codable {
  let schemaVersion = "roomplan-reference/v1"
  let revisionID: String
  let roomID: String
  let importedAt: String
  let status = "raw-only"
  let asset: LegacyReferenceAsset
  let observation: LegacyObservation

  enum CodingKeys: String, CodingKey {
    case schemaVersion = "schema_version"
    case revisionID = "revision_id"
    case roomID = "room_id"
    case importedAt = "imported_at"
    case status
    case asset
    case observation
  }
}

private struct CurrentCaptureResult: Codable {
  let draft: CaptureDraftPayload?
  let pending: CaptureDraftPayload?
  let reference: LegacyReferencePayload?
}

private struct ImportReferenceResult: Codable {
  let reference: LegacyReferencePayload?
}

private final class RoomPlanPreviewItem: NSObject, QLPreviewItem {
  let previewItemURL: URL?

  init(url: URL) {
    self.previewItemURL = url
  }
}

private struct StoredAsset {
  let descriptor: CaptureAsset
  let url: URL
}

private enum RoomPlanPluginError: LocalizedError {
  case unsupportedDevice
  case cameraPermission
  case captureAlreadyRunning
  case noCapture
  case exportFailed(String)

  var errorDescription: String? {
    switch self {
    case .unsupportedDevice:
      return "RoomPlan requires a LiDAR-equipped iPhone or iPad."
    case .cameraPermission:
      return "Camera permission is required for RoomPlan capture."
    case .captureAlreadyRunning:
      return "A RoomPlan capture is already running."
    case .noCapture:
      return "There is no active RoomPlan capture."
    case .exportFailed(let message):
      return "RoomPlan export failed: \(message)"
    }
  }
}

final class RoomPlanPlugin: Plugin, RoomCaptureSessionDelegate, QLPreviewControllerDataSource {
  private let migrationExportQueue = DispatchQueue(
    label: "sjel.roomplan.migration-export",
    qos: .utility
  )
  private var webView: WKWebView?
  private var previewItem: RoomPlanPreviewItem?
  private var captureView: RoomCaptureView?
  private var builder: RoomBuilder?
  private var captureStartedAt: Date?
  private var draftID: String?
  private var includeMesh = false
  private var captureMode = "new_room"
  private var isCapturing = false

  override func load(webview: WKWebView) {
    self.webView = webview
    scheduleMigrationExport()
  }

  @objc public func isAvailable(_ invoke: Invoke) {
    invoke.resolve(["available": RoomCaptureSession.isSupported])
  }

  @objc public func previewReference(_ invoke: Invoke) {
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      do {
        let referenceURL = try self.currentReferenceURL()
        let reference = try JSONDecoder().decode(
          LegacyReferencePayload.self,
          from: Data(contentsOf: referenceURL)
        )
        let url = try self.roomPlanDirectory()
          .appendingPathComponent("revisions", isDirectory: true)
          .appendingPathComponent(reference.revisionID, isDirectory: true)
          .appendingPathComponent("captured-room.usdz")
        guard FileManager.default.fileExists(atPath: url.path) else {
          invoke.reject("The imported RoomPlan asset is not available locally.")
          return
        }
        let controller = QLPreviewController()
        self.previewItem = RoomPlanPreviewItem(url: url)
        controller.dataSource = self
        self.topViewController()?.present(controller, animated: true)
        invoke.resolve(["status": "presented"])
      } catch {
        invoke.reject("Could not open the imported RoomPlan asset: \(error.localizedDescription)")
      }
    }
  }

  @objc public func currentCapture(_ invoke: Invoke) {
    do {
      let url = try currentDraftURL()
      let draft: CaptureDraftPayload?
      if FileManager.default.fileExists(atPath: url.path) {
        let data = try Data(contentsOf: url)
        draft = try JSONDecoder().decode(CaptureDraftPayload.self, from: data)
      } else {
        draft = nil
      }
      let pendingURL = try pendingDraftURL()
      let pending: CaptureDraftPayload?
      if FileManager.default.fileExists(atPath: pendingURL.path) {
        pending = try JSONDecoder().decode(
          CaptureDraftPayload.self,
          from: Data(contentsOf: pendingURL)
        )
      } else {
        pending = nil
      }
      let referenceURL = try currentReferenceURL()
      let reference: LegacyReferencePayload?
      if FileManager.default.fileExists(atPath: referenceURL.path) {
        reference = try JSONDecoder().decode(
          LegacyReferencePayload.self,
          from: Data(contentsOf: referenceURL)
        )
      } else {
        reference = nil
      }
      invoke.resolve(CurrentCaptureResult(draft: draft, pending: pending, reference: reference))
    } catch {
      invoke.reject("Could not load the current RoomPlan capture: \(error.localizedDescription)")
    }
  }

  @objc public func acceptPending(_ invoke: Invoke) {
    do {
      let pendingURL = try pendingDraftURL()
      guard FileManager.default.fileExists(atPath: pendingURL.path) else {
        invoke.reject("There is no pending RoomPlan revision to accept.")
        return
      }
      let data = try Data(contentsOf: pendingURL)
      _ = try JSONDecoder().decode(CaptureDraftPayload.self, from: data)
      try data.write(to: currentDraftURL(), options: [.atomic])
      try FileManager.default.removeItem(at: pendingURL)
      invoke.resolve(["status": "accepted"])
    } catch {
      invoke.reject("Could not accept the RoomPlan revision: \(error.localizedDescription)")
    }
  }

  @objc public func rejectPending(_ invoke: Invoke) {
    do {
      let pendingURL = try pendingDraftURL()
      if FileManager.default.fileExists(atPath: pendingURL.path) {
        try FileManager.default.removeItem(at: pendingURL)
      }
      invoke.resolve(["status": "rejected"])
    } catch {
      invoke.reject("Could not reject the RoomPlan revision: \(error.localizedDescription)")
    }
  }

  @objc public func importLegacyReference(_ invoke: Invoke) throws {
    do {
      let documents = try FileManager.default.url(
        for: .documentDirectory,
        in: .userDomainMask,
        appropriateFor: nil,
        create: false
      )
      let manifestURL = documents.appendingPathComponent("sync-manifest.json")
      guard FileManager.default.fileExists(atPath: manifestURL.path) else {
        invoke.resolve(ImportReferenceResult(reference: nil))
        return
      }
      let manifest = try JSONDecoder().decode(
        LegacyImportManifest.self,
        from: Data(contentsOf: manifestURL)
      )
      let referenceURL = try currentReferenceURL()
      if FileManager.default.fileExists(atPath: referenceURL.path) {
        let existing = try JSONDecoder().decode(
          LegacyReferencePayload.self,
          from: Data(contentsOf: referenceURL)
        )
        if existing.roomID == manifest.roomId && existing.asset.sha256 == manifest.expectedSha256 {
          invoke.resolve(ImportReferenceResult(reference: existing))
          return
        }
      }

      let sourceURL = documents.appendingPathComponent("captured-room.usdz")
      let observationURL = documents.appendingPathComponent("observation.json")
      guard FileManager.default.fileExists(atPath: sourceURL.path),
            FileManager.default.fileExists(atPath: observationURL.path) else {
        invoke.resolve(ImportReferenceResult(reference: nil))
        return
      }

      let data = try Data(contentsOf: sourceURL)
      let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
      guard digest == manifest.expectedSha256 else {
        throw RoomPlanPluginError.exportFailed("legacy USDZ hash does not match the sync manifest")
      }
      let observation = try JSONDecoder().decode(
        LegacyObservation.self,
        from: Data(contentsOf: observationURL)
      )
      guard observation.sha256 == digest, observation.format == "usdz" else {
        throw RoomPlanPluginError.exportFailed("legacy observation does not match the USDZ")
      }

      let revisionID = "legacy-\(digest.prefix(16))"
      let revisionDirectory = try roomPlanDirectory()
        .appendingPathComponent("revisions", isDirectory: true)
        .appendingPathComponent(revisionID, isDirectory: true)
      try FileManager.default.createDirectory(at: revisionDirectory, withIntermediateDirectories: true)
      let destinationURL = revisionDirectory.appendingPathComponent("captured-room.usdz")
      if !FileManager.default.fileExists(atPath: destinationURL.path) {
        try FileManager.default.copyItem(at: sourceURL, to: destinationURL)
      }
      let storedObservationURL = revisionDirectory.appendingPathComponent("observation.json")
      if !FileManager.default.fileExists(atPath: storedObservationURL.path) {
        try FileManager.default.copyItem(at: observationURL, to: storedObservationURL)
      }

      let reference = LegacyReferencePayload(
        revisionID: revisionID,
        roomID: manifest.roomId,
        importedAt: ISO8601DateFormatter().string(from: Date()),
        asset: LegacyReferenceAsset(
          format: "usdz",
          byteLength: UInt64(data.count),
          sha256: digest,
          storageToken: "revisions/\(revisionID)/captured-room.usdz"
        ),
        observation: observation
      )
      try JSONEncoder().encode(reference).write(to: referenceURL, options: [.atomic])
      invoke.resolve(ImportReferenceResult(reference: reference))
    } catch {
      invoke.reject("Could not import the legacy RoomPlan reference: \(error.localizedDescription)")
    }
  }

  @objc public override func checkPermissions(_ invoke: Invoke) {
    let state: String
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      state = "granted"
    case .denied, .restricted:
      state = "denied"
    case .notDetermined:
      state = "prompt"
    @unknown default:
      state = "prompt"
    }
    invoke.resolve(["camera": state])
  }

  @objc public override func requestPermissions(_ invoke: Invoke) {
    AVCaptureDevice.requestAccess(for: .video) { granted in
      invoke.resolve(["camera": granted ? "granted" : "denied"])
    }
  }

  @objc public func startCapture(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(StartCaptureArguments.self)
    guard RoomCaptureSession.isSupported else {
      invoke.reject(RoomPlanPluginError.unsupportedDevice.localizedDescription)
      return
    }
    guard !isCapturing else {
      invoke.reject(RoomPlanPluginError.captureAlreadyRunning.localizedDescription)
      return
    }

    let begin: () -> Void = { [weak self] in
      // RoomCaptureView creates an ARView internally. RoomPlan/RealityKit asserts that
      // initialization happens on the main queue, while Tauri plugin invocations may arrive
      // on a worker queue.
      DispatchQueue.main.async {
        self?.beginCapture(
          includeMesh: args.includeMesh ?? false,
          mode: args.mode ?? "new_room",
          invoke: invoke
        )
      }
    }
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      begin()
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .video) { granted in
        DispatchQueue.main.async {
          guard granted else {
            invoke.reject(RoomPlanPluginError.cameraPermission.localizedDescription)
            return
          }
          begin()
        }
      }
    default:
      invoke.reject(RoomPlanPluginError.cameraPermission.localizedDescription)
    }
  }

  @objc public func stopCapture(_ invoke: Invoke) {
    DispatchQueue.main.async { [weak self] in
      guard let self, let captureView = self.captureView, self.isCapturing else {
        invoke.reject(RoomPlanPluginError.noCapture.localizedDescription)
        return
      }
      captureView.captureSession.stop()
      invoke.resolve(["status": "processing"])
    }
  }

  @objc public func cancelCapture(_ invoke: Invoke) {
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      if self.isCapturing {
        self.captureView?.captureSession.stop()
      }
      self.tearDownCaptureView()
      invoke.resolve(["status": "cancelled"])
      try? self.trigger("capture-cancelled", data: CaptureProgressEvent(phase: "cancelled", draftID: self.draftID))
    }
  }

  private func beginCapture(includeMesh: Bool, mode: String, invoke: Invoke) {
    guard let webView else {
      invoke.reject("RoomPlan web view is not ready")
      return
    }

    self.includeMesh = includeMesh
    self.captureMode = mode == "refine_existing" ? "refine_existing" : "new_room"
    self.isCapturing = true
    self.captureStartedAt = Date()
    self.draftID = UUID().uuidString.lowercased()
    self.builder = RoomBuilder(options: [.beautifyObjects])

    let view = RoomCaptureView(frame: webView.superview?.bounds ?? UIScreen.main.bounds)
    view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.captureSession.delegate = self
    webView.superview?.insertSubview(view, aboveSubview: webView)
    self.captureView = view
    view.captureSession.run(configuration: RoomCaptureSession.Configuration())

    UIApplication.shared.isIdleTimerDisabled = true
    try? trigger("capture-progress", data: CaptureProgressEvent(phase: "capturing", draftID: draftID))
    invoke.resolve(["status": "started", "draft_id": draftID ?? ""])
  }

  func captureSession(_ session: RoomCaptureSession, didEndWith data: CapturedRoomData, error: Error?) {
    if let error {
      fail(error.localizedDescription)
      return
    }

    guard let builder else {
      fail("RoomBuilder was not initialized")
      return
    }

    try? trigger("capture-progress", data: CaptureProgressEvent(phase: "building", draftID: draftID))
    Task { [weak self] in
      do {
        let room = try await builder.capturedRoom(from: data)
        try await self?.finishCapture(room: room)
      } catch {
        self?.fail(error.localizedDescription)
      }
    }
  }

  private func finishCapture(room: CapturedRoom) async throws {
    guard let draftID else { throw RoomPlanPluginError.noCapture }
    let started = captureStartedAt
    let finished = Date()
    let directory = try captureDirectory(for: draftID)
    let assets = try export(room: room, draftID: draftID, directory: directory)
    // Apple's own serialization, kept beside the draft as raw evidence (PRD §10, "Native semantic
    // capture": preserve CapturedRoom, not only the normalized draft). It is not a draft asset:
    // whether it becomes one is A2's decision, and the asset contract only admits USDZ today.
    // A failure here must not lose the capture, so it is logged rather than thrown.
    do {
      try JSONEncoder().encode(room).write(
        to: directory.appendingPathComponent("captured-room.json"), options: [.atomic])
    } catch {
      NSLog("RoomPlan: captured-room.json not written: \(error.localizedDescription)")
    }
    let payload = CaptureDraftPayload(
      draftID: draftID,
      createdAt: ISO8601DateFormatter().string(from: finished),
      source: CaptureSource(
        roomplanVersion: ProcessInfo.processInfo.operatingSystemVersionString,
        appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
      ),
      room: map(room),
      assets: assets.map(\.descriptor),
      provenance: CaptureProvenance(
        captureMode: self.captureMode,
        parentRevisionID: self.captureMode == "refine_existing" ? try self.currentRevisionID() : nil,
        captureStartedAt: started.map { ISO8601DateFormatter().string(from: $0) },
        captureFinishedAt: ISO8601DateFormatter().string(from: finished)
      )
    )

    let jsonURL = directory.appendingPathComponent("draft.json")
    let json = try JSONEncoder().encode(payload)
    try json.write(to: jsonURL, options: [.atomic])
    if self.captureMode == "refine_existing" {
      try json.write(to: pendingDraftURL(), options: [.atomic])
    } else {
      try json.write(to: currentDraftURL(), options: [.atomic])
    }
    scheduleMigrationExport()
    try await MainActor.run {
      self.isCapturing = false
      self.tearDownCaptureView()
      try self.trigger("capture-completed", data: CaptureCompletedEvent(draft: payload))
    }
  }

  private func export(room: CapturedRoom, draftID: String, directory: URL) throws -> [StoredAsset] {
    var outputs: [StoredAsset] = []
    let parametricURL = directory.appendingPathComponent("parametric.usdz")
    do {
      try room.export(to: parametricURL, exportOptions: .parametric)
      outputs.append(try asset(url: parametricURL, id: "\(draftID)-parametric", role: "parametric", token: "\(draftID)/parametric.usdz"))
    } catch {
      throw RoomPlanPluginError.exportFailed(error.localizedDescription)
    }

    if includeMesh {
      let meshURL = directory.appendingPathComponent("mesh.usdz")
      do {
        try room.export(to: meshURL, exportOptions: .mesh)
        outputs.append(try asset(url: meshURL, id: "\(draftID)-mesh", role: "mesh", token: "\(draftID)/mesh.usdz"))
      } catch {
        throw RoomPlanPluginError.exportFailed(error.localizedDescription)
      }
    }
    return outputs
  }

  private func asset(url: URL, id: String, role: String, token: String) throws -> StoredAsset {
    let data = try Data(contentsOf: url)
    let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    return StoredAsset(
      descriptor: CaptureAsset(
        assetID: id,
        role: role,
        byteLength: UInt64(data.count),
        sha256: digest,
        storageToken: token
      ),
      url: url
    )
  }

  private func roomPlanDirectory() throws -> URL {
    let directory = try FileManager.default.url(
      for: .applicationSupportDirectory,
      in: .userDomainMask,
      appropriateFor: nil,
      create: true
    ).appendingPathComponent("RoomPlan", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory
  }

  private func currentDraftURL() throws -> URL {
    try roomPlanDirectory().appendingPathComponent("current.json")
  }

  private func pendingDraftURL() throws -> URL {
    try roomPlanDirectory().appendingPathComponent("pending.json")
  }

  private func currentReferenceURL() throws -> URL {
    try roomPlanDirectory().appendingPathComponent("current-reference.json")
  }

  private func currentRevisionID() throws -> String? {
    let draftURL = try currentDraftURL()
    if FileManager.default.fileExists(atPath: draftURL.path) {
      let draft = try JSONDecoder().decode(
        CaptureDraftPayload.self,
        from: Data(contentsOf: draftURL)
      )
      return draft.draftID
    }
    let referenceURL = try currentReferenceURL()
    if FileManager.default.fileExists(atPath: referenceURL.path) {
      let reference = try JSONDecoder().decode(
        LegacyReferencePayload.self,
        from: Data(contentsOf: referenceURL)
      )
      return reference.revisionID
    }
    return nil
  }

  private func captureDirectory(for draftID: String) throws -> URL {
    let directory = try roomPlanDirectory().appendingPathComponent(draftID, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory
  }

  private func scheduleMigrationExport() {
    migrationExportQueue.async { [weak self] in
      guard let self else { return }
      do {
        try self.stageLegacyRoomPlanCaptures()
      } catch {
        NSLog("RoomPlan: local migration export failed: \(error.localizedDescription)")
      }
    }
  }

  private func stageLegacyRoomPlanCaptures() throws {
    let fileManager = FileManager.default
    let source = try roomPlanDirectory()
    let documents = try fileManager.url(
      for: .documentDirectory,
      in: .userDomainMask,
      appropriateFor: nil,
      create: true
    )
    let destination = documents.appendingPathComponent(
      "RoomPlan-GPUI-Migration",
      isDirectory: true
    )
    try fileManager.createDirectory(at: destination, withIntermediateDirectories: true)
    let ready = destination.appendingPathComponent(".ready")
    try? fileManager.removeItem(at: ready)

    let directories = try fileManager.contentsOfDirectory(
      at: source,
      includingPropertiesForKeys: [.isDirectoryKey],
      options: [.skipsHiddenFiles]
    )
    for capture in directories where
      (try? capture.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
      let draft = capture.appendingPathComponent("draft.json")
      let asset = capture.appendingPathComponent("parametric.usdz")
      let draftID = capture.lastPathComponent
      guard fileManager.fileExists(atPath: draft.path),
            fileManager.fileExists(atPath: asset.path),
            !draftID.isEmpty,
            draftID.utf8.allSatisfy({
              ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122) ||
                ($0 >= 48 && $0 <= 57) || $0 == 45 || $0 == 95
            }) else {
        continue
      }
      let target = destination.appendingPathComponent(draftID, isDirectory: true)
      guard !fileManager.fileExists(atPath: target.path) else { continue }
      let staging = destination.appendingPathComponent(
        ".\(draftID).staging-\(UUID().uuidString)",
        isDirectory: true
      )
      do {
        try fileManager.copyItem(at: capture, to: staging)
        try fileManager.moveItem(at: staging, to: target)
      } catch {
        try? fileManager.removeItem(at: staging)
        throw error
      }
    }
    try Data("ready".utf8).write(to: ready, options: [.atomic])
  }

  private func map(_ room: CapturedRoom) -> CapturedRoomPayload {
    var surfaces = room.walls + room.doors + room.windows
    if #available(iOS 17.0, *) {
      surfaces += room.floors
    }
    return CapturedRoomPayload(
      id: room.identifier.uuidString.lowercased(),
      surfaces: surfaces.map { mapSurface($0) },
      openings: room.openings.map { mapSurface($0) },
      objects: room.objects.map { mapObject($0) }
    )
  }

  private func mapSurface(_ surface: CapturedRoom.Surface) -> RoomElement {
    RoomElement(
      id: "surface-\(surface.identifier.uuidString.lowercased())",
      category: String(describing: surface.category),
      dimensionsM: [surface.dimensions.x, surface.dimensions.y, surface.dimensions.z],
      transform: matrix(surface.transform),
      confidence: confidence(surface.confidence),
      sourceID: surface.identifier.uuidString.lowercased()
    )
  }

  private func mapObject(_ object: CapturedRoom.Object) -> RoomElement {
    RoomElement(
      id: "object-\(object.identifier.uuidString.lowercased())",
      category: String(describing: object.category),
      dimensionsM: [object.dimensions.x, object.dimensions.y, object.dimensions.z],
      transform: matrix(object.transform),
      confidence: confidence(object.confidence),
      sourceID: object.identifier.uuidString.lowercased()
    )
  }

  private func confidence(_ value: CapturedRoom.Confidence) -> String {
    switch value {
    case .high:
      return "high"
    case .medium:
      return "medium"
    case .low:
      return "low"
    @unknown default:
      return "low"
    }
  }

  private func matrix(_ value: simd_float4x4) -> [Float] {
    [
      value.columns.0.x, value.columns.0.y, value.columns.0.z, value.columns.0.w,
      value.columns.1.x, value.columns.1.y, value.columns.1.z, value.columns.1.w,
      value.columns.2.x, value.columns.2.y, value.columns.2.z, value.columns.2.w,
      value.columns.3.x, value.columns.3.y, value.columns.3.z, value.columns.3.w,
    ]
  }

  private func fail(_ message: String) {
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      self.isCapturing = false
      self.tearDownCaptureView()
      self.trigger("capture-failed", data: ["message": message, "draft_id": self.draftID ?? ""])
    }
  }

  private func topViewController() -> UIViewController? {
    var controller = webView?.window?.rootViewController
    while let presented = controller?.presentedViewController {
      controller = presented
    }
    if let navigation = controller as? UINavigationController {
      return navigation.visibleViewController ?? navigation
    }
    if let tab = controller as? UITabBarController {
      return tab.selectedViewController ?? tab
    }
    return controller
  }

  func numberOfPreviewItems(in controller: QLPreviewController) -> Int {
    previewItem == nil ? 0 : 1
  }

  func previewController(
    _ controller: QLPreviewController,
    previewItemAt index: Int
  ) -> QLPreviewItem {
    previewItem ?? RoomPlanPreviewItem(url: URL(fileURLWithPath: "/"))
  }

  private func tearDownCaptureView() {
    captureView?.removeFromSuperview()
    captureView = nil
    builder = nil
    UIApplication.shared.isIdleTimerDisabled = false
  }
}

@_cdecl("init_plugin_roomplan")
func initPlugin() -> Plugin {
  RoomPlanPlugin()
}
