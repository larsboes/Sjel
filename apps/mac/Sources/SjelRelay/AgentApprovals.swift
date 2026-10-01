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

/// Reads the waiting writes from the shell and sends the owner's decision
/// (`GET /api/sjel-status/agent`, `POST /api/sjel-status/agent/approvals/{id}`).
/// Authenticated with the deployment token from the login Keychain, like `DashboardLogin`.
public struct AgentApprovals: Sendable {
    public let baseURL: URL
    private let readToken: @Sendable () -> String?

    public init(
        baseURL: URL = URL(string: "http://127.0.0.1:8082")!,
        readToken: @escaping @Sendable () -> String? = { DashboardLogin.keychainToken() }
    ) {
        self.baseURL = baseURL
        self.readToken = readToken
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
        struct View: Decodable { let pending: [AgentApproval]? }
        return (try? JSONDecoder().decode(View.self, from: data))?.pending ?? []
    }

    /// The waiting writes, or an empty list when the shell or the token is not there.
    public func pending(session: URLSession = .shared) async -> [AgentApproval] {
        guard let request = pendingRequest(),
              let (data, response) = try? await session.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200
        else { return [] }
        return Self.pending(fromAgentView: data)
    }

    /// `true` when the shell recorded the decision.
    public func decide(id: String, allow: Bool, session: URLSession = .shared) async -> Bool {
        guard let request = decisionRequest(id: id, allow: allow),
              let (_, response) = try? await session.data(for: request)
        else { return false }
        return (response as? HTTPURLResponse)?.statusCode == 200
    }
}
