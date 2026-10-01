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

    /// The token, read from the login Keychain on demand and never kept.
    public static func keychainToken() -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: NSUserName(),
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let token = String(data: data, encoding: .utf8)?
                  .trimmingCharacters(in: .whitespacesAndNewlines),
              !token.isEmpty
        else { return nil }
        return token
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

    /// Asks the shell for a ticket and returns the URL that logs the browser in.
    public func loginURL(session: URLSession = .shared) async throws(Failure) -> URL {
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
        return url
    }
}
