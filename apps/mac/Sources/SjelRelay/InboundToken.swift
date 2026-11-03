import Foundation
import Security

/// The deployment's inbound token in the login Keychain, written by `tools/setup-inbound-auth.sh`.
public enum InboundToken {
    /// The Keychain item `tools/setup-inbound-auth.sh` writes.
    public static let keychainService = "sjel-inbound-token"

    /// What one Keychain read found. `refused` is the owner's Deny (or a cancelled prompt),
    /// which a caller must not answer by asking again.
    public enum KeychainRead: Equatable, Sendable {
        case token(String)
        case missing
        case refused
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
}

/// The token for a caller that runs on a timer. Before this, the approvals poll read the
/// Keychain every five seconds, and once `tools/setup-inbound-auth.sh` had created the item,
/// every read showed the access prompt again, and a Deny came back five seconds later
/// (2026-10-01). It keeps a token it read, stops reading after a refusal until
/// `forget()`, and keeps reading quietly while the item is missing, which shows no prompt.
public final class CachedToken: @unchecked Sendable {
    private let lock = NSLock()
    private let read: @Sendable () -> InboundToken.KeychainRead
    private var token: String?
    private var refused = false

    public init(read: @escaping @Sendable () -> InboundToken.KeychainRead = { InboundToken.readKeychain() }) {
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
