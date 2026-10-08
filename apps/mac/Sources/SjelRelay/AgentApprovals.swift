import Foundation

/// One agent write that waits for the owner (ISA F10, ask mode).
public struct AgentApproval: Sendable, Equatable, Identifiable, Decodable {
    public let id: String
    public let capability: String
    public let method: String
    public let path: String
    public let preview: String

    public init(id: String, capability: String, method: String, path: String, preview: String) {
        self.id = id
        self.capability = capability
        self.method = method
        self.path = path
        self.preview = preview
    }

    /// One line for a person: what the agent wants to do, and where.
    public var summary: String { "\(capability): \(method) \(path)" }
}

/// One call an agent made through the gate (`AgentCall` in `libs/sjel-server/src/agent_policy.rs`).
public struct AgentCallRecord: Sendable, Equatable, Decodable, Identifiable {
    public let at: Int
    public let capability: String
    public let method: String
    public let path: String
    public let status: Int
    /// "read" for a plain read. Anything else (a write, a refusal) is worth a look.
    public let decision: String

    public var id: String { "\(at)\(capability)\(method)\(path)" }
    public var date: Date { Date(timeIntervalSince1970: TimeInterval(at)) }
    public var isRead: Bool { decision == "read" }
}

/// The parts of `GET /api/sjel-status/agent` the menu bar shows.
public struct AgentView: Sendable, Equatable {
    public var pending: [AgentApproval] = []
    /// Newest first, at most 50 (`recent_calls(50)` in `capabilities/sjel-status/src/status/agent.rs`).
    public var calls: [AgentCallRecord] = []

    public init(pending: [AgentApproval] = [], calls: [AgentCallRecord] = []) {
        self.pending = pending
        self.calls = calls
    }

    public static func decode(_ data: Data) -> AgentView {
        struct Wire: Decodable {
            let pending: [AgentApproval]?
            let calls: [AgentCallRecord]?
        }
        let wire = try? JSONDecoder().decode(Wire.self, from: data)
        return AgentView(pending: wire?.pending ?? [], calls: wire?.calls ?? [])
    }
}

/// Reads the waiting writes from the shell and sends the owner's decision
/// (`GET /api/sjel-status/agent`, `POST /api/sjel-status/agent/approvals/{id}`).
/// Authenticated with the deployment token from the login Keychain, like `DashboardLogin`.
public struct AgentApprovals: Sendable {
    public let baseURL: URL
    private let readToken: @Sendable () -> String?
    private let forgetToken: @Sendable () -> Void

    /// Polled every five seconds, so the token comes from a `CachedToken`: one Keychain read,
    /// not one prompt per poll.
    public init(baseURL: URL = URL(string: "http://127.0.0.1:8082")!, token: CachedToken = CachedToken()) {
        self.init(baseURL: baseURL, readToken: { token.get() }, forgetToken: { token.forget() })
    }

    init(
        baseURL: URL = URL(string: "http://127.0.0.1:8082")!,
        readToken: @escaping @Sendable () -> String?,
        forgetToken: @escaping @Sendable () -> Void = {}
    ) {
        self.baseURL = baseURL
        self.readToken = readToken
        self.forgetToken = forgetToken
    }

    private func request(_ path: String, method: String, body: Data? = nil) -> URLRequest? {
        guard let token = readToken() else { return nil }
        var request = URLRequest(url: baseURL.appendingPathComponent(path))
        request.httpMethod = method
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.timeoutInterval = 5
        if let body {
            request.httpBody = body
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        return request
    }

    public func pendingRequest() -> URLRequest? {
        request("/api/sjel-status/agent", method: "GET")
    }

    public func decisionRequest(id: String, allow: Bool) -> URLRequest? {
        let safe = !id.isEmpty && id.allSatisfy(\.isHexDigit)
        guard safe else { return nil }
        let body = try? JSONSerialization.data(withJSONObject: ["allow": allow])
        return request("/api/sjel-status/agent/approvals/\(id)", method: "POST", body: body)
    }

    /// The `pending` list of the agent view; empty when the answer has none.
    public static func pending(fromAgentView data: Data) -> [AgentApproval] {
        AgentView.decode(data).pending
    }

    /// The waiting writes and the recent calls, or an empty view when the shell or the token
    /// is not there.
    public func view(session: URLSession = .shared) async -> AgentView {
        guard let request = pendingRequest(),
              let (data, response) = try? await session.data(for: request)
        else { return AgentView() }
        let status = (response as? HTTPURLResponse)?.statusCode
        // A 401 means the kept token is no longer the deployment's: read it again next poll.
        if status == 401 { forgetToken() }
        guard status == 200 else { return AgentView() }
        return AgentView.decode(data)
    }

    /// `true` when the shell recorded the decision.
    public func decide(id: String, allow: Bool, session: URLSession = .shared) async -> Bool {
        guard let request = decisionRequest(id: id, allow: allow),
              let (_, response) = try? await session.data(for: request)
        else { return false }
        return (response as? HTTPURLResponse)?.statusCode == 200
    }
}
