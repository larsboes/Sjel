import Foundation

public struct SjelNodeStatus: Sendable, Equatable {
    public let isReachable: Bool
    public let nodeName: String
    public let version: String
    public let latencyMs: Double
    public let lastChecked: Date

    public init(
        isReachable: Bool,
        nodeName: String = "Sjel Local Node",
        version: String = "v0.0.1",
        latencyMs: Double = 0.0,
        lastChecked: Date = Date()
    ) {
        self.isReachable = isReachable
        self.nodeName = nodeName
        self.version = version
        self.latencyMs = latencyMs
        self.lastChecked = lastChecked
    }
}

/// Bridge connecting the native macOS companion to the local Sjel daemon.
public final class NodeBridge: Sendable {
    public let baseURL: URL
    private let session: URLSession

    /// sjel-status, the shell (`capabilities/sjel-status/service.toml`). This said 8080, where
    /// nothing listens, so the menu showed "offline" whatever the shell's state.
    public init(baseURL: URL = URL(string: "http://127.0.0.1:8082")!) {
        self.baseURL = baseURL
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 2.0
        self.session = URLSession(configuration: config)
    }

    /// Checks the health of the local Sjel server.
    public func checkHealth() async -> SjelNodeStatus {
        let healthURL = baseURL.appendingPathComponent("health")
        let start = CFAbsoluteTimeGetCurrent()
        do {
            let (data, response) = try await session.data(from: healthURL)
            let latency = (CFAbsoluteTimeGetCurrent() - start) * 1000.0
            guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
                return SjelNodeStatus(isReachable: false, latencyMs: latency)
            }

            // sjel-status answers `{"ok": true, "service": "sjel-status"}`.
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               json["ok"] as? Bool == true {
                let version = (json["version"] as? String) ?? "v0.0.1"
                return SjelNodeStatus(isReachable: true, version: version, latencyMs: latency)
            }

            return SjelNodeStatus(isReachable: true, latencyMs: latency)
        } catch {
            let latency = (CFAbsoluteTimeGetCurrent() - start) * 1000.0
            return SjelNodeStatus(isReachable: false, latencyMs: latency)
        }
    }
}
