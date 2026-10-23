import AppKit

/// Whether the installed Sjel Desktop is behind the Sjel checkout.
///
/// `apps/gpui/install` writes the commit it built into the bundle's Info.plist as
/// `SjelBuildCommit`, the same value the desktop app compiles in (`apps/gpui/build.rs`). This
/// counts the checkout's commits after it. Uncommitted changes do not count: agents edit the
/// checkout all day (`apps/gpui/src/update.rs`, `is_update_available`).
struct DesktopUpdate: Equatable {
    let behind: Int
    /// The subject of the newest commit.
    let newest: String

    /// The checkout the desktop app is built from, as in `updateDesktopApp`.
    static var repo: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Developer/Sjel")
    }

    /// Nil when the app is not installed, carries no commit, or is up to date.
    static func check() async -> DesktopUpdate? {
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "dev.sjel.desktop"),
              // Read the file, not `Bundle(url:)`: Bundle caches its Info.plist across a reinstall.
              let info = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist")),
              let installed = info["SjelBuildCommit"] as? String, installed != "dev"
        else { return nil }
        return await Task.detached {
            guard let count = git(["rev-list", "--count", "\(installed)..HEAD"]),
                  let behind = Int(count), behind > 0
            else { return nil }
            return DesktopUpdate(behind: behind, newest: git(["log", "-1", "--format=%s"]) ?? "")
        }.value
    }

    private static func git(_ arguments: [String]) -> String? {
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
