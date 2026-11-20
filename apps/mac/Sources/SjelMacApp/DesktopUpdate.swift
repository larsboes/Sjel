import AppKit

/// An update from the installed desktop bundle's source checkout.
struct DesktopUpdate: Equatable {
    let behind: Int
    let newest: String

    struct Source: Sendable {
        let repo: URL
        let revision: String
        let environment: [String: String]
    }

    @MainActor
    static func installedSource() -> Source? {
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "dev.sjel.desktop"),
              // Read the file because Bundle caches metadata across reinstalls.
              let info = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist"))
        else { return nil }
        return source(from: info)
    }

    static func source(from info: NSDictionary) -> Source? {
        guard let revision = info["SjelBuildCommit"] as? String,
              revision.range(of: #"\A[0-9a-fA-F]{7,40}\z"#, options: .regularExpression) != nil
        else { return nil }
        let settings = info["LSEnvironment"] as? [String: String] ?? [:]
        let home = FileManager.default.homeDirectoryForCurrentUser
        let repo = settings["SJEL_DESKTOP_SOURCE_ROOT"].map { URL(fileURLWithPath: $0) }
            ?? home.appendingPathComponent("Developer/Sjel")
        guard FileManager.default.isExecutableFile(atPath: repo.appendingPathComponent("apps/gpui/install").path)
        else { return nil }

        var environment = ProcessInfo.processInfo.environment
        environment["PATH"] = "\(home.path)/.local/bin:\(home.path)/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        for key in ["SJEL_DESKTOP_BUILD_PROFILE", "SJEL_GPUI_NATIVE_MAPS", "SJEL_PERSONAL_ROOT"] {
            environment[key] = settings[key]
        }
        return Source(repo: repo, revision: revision, environment: environment)
    }

    @MainActor
    static func check() async -> DesktopUpdate? {
        guard let source = installedSource() else { return nil }
        return await Task.detached { available(from: source) }.value
    }

    static func available(from source: Source) -> DesktopUpdate? {
        guard let target = git(["rev-parse", "HEAD"], in: source.repo),
              git(["merge-base", "--is-ancestor", source.revision, target], in: source.repo) != nil,
              let count = git(["rev-list", "--count", "\(source.revision)..\(target)"], in: source.repo),
              let behind = Int(count), behind > 0
        else { return nil }
        return DesktopUpdate(behind: behind, newest: git(["log", "-1", "--format=%s", target], in: source.repo) ?? "")
    }

    static func installUpdate(from source: Source) -> String {
        guard git(["merge-base", "--is-ancestor", source.revision, "HEAD"], in: source.repo) != nil else {
            return "The desktop source checkout does not contain the installed revision."
        }
        guard available(from: source) != nil else {
            return "Sjel Desktop is up to date."
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/bash")
        process.arguments = [source.repo.appendingPathComponent("apps/gpui/install").path]
        process.currentDirectoryURL = source.repo
        process.environment = source.environment
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            if process.terminationStatus == 0 { return "Sjel Desktop updated." }
            let lines = String(decoding: data, as: UTF8.self).split(whereSeparator: \.isNewline)
            let detail = lines.last { $0.contains("error:") }
                ?? lines.last { $0.contains(": ") }
            if let detail { return String(detail) }
            return "Desktop update failed (code \(process.terminationStatus))."
        } catch {
            return "Desktop update error: \(error.localizedDescription)"
        }
    }

    private static func git(_ arguments: [String], in repo: URL) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-C", repo.path] + arguments
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        guard (try? process.run()) != nil else { return nil }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
