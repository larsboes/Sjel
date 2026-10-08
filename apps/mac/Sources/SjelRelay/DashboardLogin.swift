import Foundation
import Security

/// Opens the dashboard in the browser, logged in (ISA ISC-45).
///
/// The shell at 127.0.0.1:8082 requires a credential, and a browser carries none. This app
/// reads the deployment's inbound token from the login Keychain, trades it for a single-use
/// ticket (`POST /api/sjel-status/session/ticket`), and opens the browser at the URL the shell
/// answers with. The shell turns the ticket into a 30-day session cookie
/// (`capabilities/sjel-status/src/session.rs`). The token never reaches the browser.
public struct DashboardLogin: Sendable {
    public enum Failure: Error, Equatable, Sendable {
        /// No token in the Keychain: `tools/setup-inbound-auth.sh` has not run on this Mac.
        case notSetUp
        /// The shell is not answering.
        case unreachable
        /// The shell answered, but not with a ticket. Carries the HTTP status.
        case refused(Int)
    }

    /// The Keychain item `tools/setup-inbound-auth.sh` writes.
    public static let keychainService = "sjel-inbound-token"
    public static let ticketPath = "/api/sjel-status/session/ticket"
    public static let openPath = "/session/open"

    public let baseURL: URL
    private let readToken: @Sendable () -> String?

    public init(
        baseURL: URL = URL(string: "http://127.0.0.1:8082")!,
        readToken: @escaping @Sendable () -> String? = { DashboardLogin.keychainToken() }
    ) {
        self.baseURL = baseURL
        self.readToken = readToken
    }

    /// What one Keychain read found. `refused` is the owner's Deny (or a cancelled prompt),
    /// which a caller must not answer by asking again.
    public enum KeychainRead: Equatable, Sendable {
        case token(String)
        case missing
        case refused
    }

    /// The token, read from the login Keychain on demand and never kept.
    public static func keychainToken() -> String? {
        if case .token(let token) = readKeychain() { return token }
        return nil
    }

    /// One Keychain read. Each call can show the macOS access prompt when this app is not on
    /// the item's access list, so a caller on a timer goes through `CachedToken` instead.
    public static func readKeychain() -> KeychainRead {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: NSUserName(),
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecUserCanceled || status == errSecAuthFailed { return .refused }
        guard status == errSecSuccess,
              let data = item as? Data,
              let token = String(data: data, encoding: .utf8)?
                  .trimmingCharacters(in: .whitespacesAndNewlines),
              !token.isEmpty
        else { return .missing }
        return .token(token)
    }


    /// The ticket request, or `nil` when there is no token to send.
    public func ticketRequest() -> URLRequest? {
        guard let token = readToken() else { return nil }
        var request = URLRequest(url: baseURL.appendingPathComponent(Self.ticketPath))
        request.httpMethod = "POST"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.timeoutInterval = 5
        return request
    }

    /// The URL to open, from the shell's answer. Only a path under `/session/open` on this
    /// same shell is accepted, so a wrong answer cannot send the browser anywhere else.
    public func openURL(fromTicketResponse data: Data) -> URL? {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let path = json["open"] as? String,
              path.hasPrefix(Self.openPath + "?"),
              let url = URL(string: path, relativeTo: baseURL)?.absoluteURL,
              url.host == baseURL.host, url.port == baseURL.port
        else { return nil }
        return url
    }

    /// Asks the shell for a ticket and returns the URL that logs the browser in. `next` is
    /// the dashboard path to land on; the shell accepts only a path on its own origin
    /// (`landing` in `capabilities/sjel-status/src/session.rs`).
    public func loginURL(next: String? = nil, session: URLSession = .shared) async throws(Failure) -> URL {
        guard let request = ticketRequest() else { throw .notSetUp }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw .unreachable
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200, let url = openURL(fromTicketResponse: data) else {
            throw .refused(status)
        }
        return Self.landing(url, next: next)
    }

    static func landing(_ url: URL, next: String?) -> URL {
        guard let next, var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return url }
        parts.queryItems = (parts.queryItems ?? []) + [URLQueryItem(name: "next", value: next)]
        return parts.url ?? url
    }
}

/// The token for a caller that runs on a timer. Before this, the approvals poll read the
/// Keychain every five seconds, and once `tools/setup-inbound-auth.sh` had created the item,
/// every read showed the access prompt again, and a Deny came back five seconds later
/// (2026-10-01). It keeps a token it read, stops reading after a refusal until
/// `forget()`, and keeps reading quietly while the item is missing, which shows no prompt.
public final class CachedToken: @unchecked Sendable {
    private let lock = NSLock()
    private let read: @Sendable () -> DashboardLogin.KeychainRead
    private var token: String?
    private var refused = false

    public init(read: @escaping @Sendable () -> DashboardLogin.KeychainRead = { DashboardLogin.readKeychain() }) {
        self.read = read
    }

    public func get() -> String? {
        lock.lock()
        defer { lock.unlock() }
        if let token { return token }
        if refused { return nil }
        switch read() {
        case .token(let value): token = value
        case .refused: refused = true
        case .missing: break
        }
        return token
    }

    /// Drops the kept token and any refusal: the shell answered 401 (the token was rotated),
    /// or the owner chose to try again.
    public func forget() {
        lock.lock()
        defer { lock.unlock() }
        token = nil
        refused = false
    }
}
