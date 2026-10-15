import Foundation

/// What the shell says about itself and every capability with a health surface
/// (`GET /api/sjel-status/health`, `AxonStatusHealth` in
/// `capabilities/sjel-status/src/status/registry.rs`).
public struct NodeHealth: Sendable, Equatable, Decodable {
    public struct Capability: Sendable, Equatable, Decodable {
        public let up: Bool
        /// The machine keeps it running (`autostart = "true"`). Anything else starts on
        /// demand, so stopped is normal for it. A shell from before the field counts every
        /// capability as kept running, the old reading.
        public let autostart: Bool

        public init(up: Bool, autostart: Bool = true) {
            self.up = up
            self.autostart = autostart
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            up = try c.decode(Bool.self, forKey: .up)
            autostart = try c.decodeIfPresent(Bool.self, forKey: .autostart) ?? true
        }

        enum CodingKeys: String, CodingKey { case up, autostart }
    }

    public let ok: Bool
    public let version: String
    public let uptimeSeconds: UInt64
    public let capabilities: [String: Capability]

    enum CodingKeys: String, CodingKey {
        case ok, version, capabilities
        case uptimeSeconds = "uptime_seconds"
    }

    /// Names of the kept-running capabilities that did not answer, sorted so the list does not
    /// reorder between polls. An on-demand capability that is stopped is not down
    /// (ISA-archive.md DSH-1): on 2026-10-08 four of those kept the "needs you" dot on.
    public var down: [String] {
        capabilities.filter { $0.value.autostart && !$0.value.up }.map(\.key).sorted()
    }

    /// The order the menu lists every capability in: by name, whatever is up. A start or a stop
    /// changes a row's dot and button, never its place.
    public var rowOrder: [String] { capabilities.keys.sorted() }
}

public enum NodeStatus: Sendable, Equatable {
    /// Nothing answered on the shell's port.
    case offline
    /// The shell answered, but this Mac holds no token, or the shell refused it, so only
    /// "alive" is known.
    case alive
    case known(NodeHealth)
}

/// Bridge connecting the native macOS companion to the local Sjel daemon.
public final class NodeBridge: Sendable {
    public let baseURL: URL
    private let session: URLSession
    private let token: CachedToken

    /// sjel-status, the shell (`capabilities/sjel-status/service.toml`). This said 8080, where
    /// nothing listens, so the menu showed "offline" whatever the shell's state.
    public init(baseURL: URL = URL(string: "http://127.0.0.1:8082")!, token: CachedToken = CachedToken()) {
        self.baseURL = baseURL
        self.token = token
        let config = URLSessionConfiguration.ephemeral
        // The authenticated health polls every capability, so it is slower than `/health`.
        config.timeoutIntervalForRequest = 10.0
        self.session = URLSession(configuration: config)
    }

    /// The open `/health` only proves the shell process is alive: it answered `ok` while nine
    /// of 22 capabilities were down (2026-10-08). The authenticated health says which.
    public func status() async -> NodeStatus {
        if let health = await authenticatedHealth() { return .known(health) }
        guard let (_, response) = try? await session.data(from: baseURL.appendingPathComponent("health")),
              (response as? HTTPURLResponse)?.statusCode == 200
        else { return .offline }
        return .alive
    }

    private func authenticatedHealth() async -> NodeHealth? {
        guard let bearer = token.get() else { return nil }
        var request = URLRequest(url: baseURL.appendingPathComponent("api/sjel-status/health"))
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        guard let (data, response) = try? await session.data(for: request),
              let http = response as? HTTPURLResponse
        else { return nil }
        if http.statusCode == 401 { token.forget() }
        guard http.statusCode == 200 else { return nil }
        return try? JSONDecoder().decode(NodeHealth.self, from: data)
    }

    /// Starts or stops one capability through the shell
    /// (`capabilities/sjel-status/src/status/lifecycle.rs`). Start is `resume`, so it also
    /// clears the hold a stop leaves. Returns nil on success, or the words to show.
    public func setRunning(_ name: String, _ running: Bool) async -> String? {
        guard let request = lifecycleRequest(name, running) else { return "Sjel has no token on this Mac." }
        guard let (data, response) = try? await session.data(for: request) else {
            return "Sjel did not answer."
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        if status == 401 { token.forget() }
        if status == 200 { return nil }
        let error = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
        return error ?? "\(running ? "Start" : "Stop") \(name) failed (\(status))."
    }

    /// Only a registry name reaches the shell: letters, digits and dashes, nothing a path
    /// could be built from.
    func lifecycleRequest(_ name: String, _ running: Bool) -> URLRequest? {
        let safe = !name.isEmpty && name.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" }
        guard safe, let bearer = token.get() else { return nil }
        let action = running ? "start" : "stop"
        var request = URLRequest(url: baseURL.appendingPathComponent("api/sjel-status/capabilities/\(name)/\(action)"))
        request.httpMethod = "POST"
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        // A start waits for the service to come up.
        request.timeoutInterval = 60
        return request
    }

    /// Today's and tomorrow's calendar entries through the shell's proxy, or nil when the
    /// calendar did not answer.
    public func calendarToday(now: Date = Date()) async -> [CalendarEntry]? {
        guard let bearer = token.get() else { return nil }
        let range = Today.range(now: now)
        var parts = URLComponents(url: baseURL.appendingPathComponent("calendar/api/entries"), resolvingAgainstBaseURL: false)
        parts?.queryItems = [URLQueryItem(name: "from", value: range.from), URLQueryItem(name: "to", value: range.to)]
        guard let url = parts?.url else { return nil }
        var request = URLRequest(url: url)
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        guard let (data, response) = try? await session.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return try? JSONDecoder().decode([CalendarEntry].self, from: data)
    }
}
