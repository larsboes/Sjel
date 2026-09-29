import Foundation
import CryptoKit
import Security

/// Key management for local device encryption keys.
/// Keys remain strictly on the user's paired devices and never reach the cloud.
public final class RelayKeyManager: @unchecked Sendable {
    public static let shared = RelayKeyManager()

    private let serviceName = "org.sjel.relay.keys"
    private var inMemoryKeys: [UUID: SymmetricKey] = [:]
    private var primaryKeyId: UUID?
    private let lock = NSLock()

    public init() {}

    /// Generates a new 256-bit symmetric key and registers it as the primary key.
    @discardableResult
    public func generateAndStoreKey(keyId: UUID = UUID(), persistToKeychain: Bool = false) throws -> (UUID, SymmetricKey) {
        let key = SymmetricKey(size: .bits256)
        lock.lock()
        defer { lock.unlock() }

        inMemoryKeys[keyId] = key
        primaryKeyId = keyId

        if persistToKeychain {
            try saveToKeychain(keyId: keyId, key: key)
        }

        return (keyId, key)
    }

    /// Stores an existing key into the manager.
    public func registerKey(keyId: UUID, key: SymmetricKey, makePrimary: Bool = true) {
        lock.lock()
        defer { lock.unlock() }
        inMemoryKeys[keyId] = key
        if makePrimary || primaryKeyId == nil {
            primaryKeyId = keyId
        }
    }

    /// Retrieves the current primary key.
    public func getPrimaryKey() throws -> (UUID, SymmetricKey) {
        lock.lock()
        defer { lock.unlock() }

        if let id = primaryKeyId, let key = inMemoryKeys[id] {
            return (id, key)
        }

        // Generate an ephemeral key if none exists
        let newId = UUID()
        let newKey = SymmetricKey(size: .bits256)
        inMemoryKeys[newId] = newKey
        primaryKeyId = newId
        return (newId, newKey)
    }

    /// Retrieves a key by its UUID.
    public func getKey(keyId: UUID) -> SymmetricKey? {
        lock.lock()
        defer { lock.unlock() }
        return inMemoryKeys[keyId]
    }

    private func saveToKeychain(keyId: UUID, key: SymmetricKey) throws {
        let keyData = key.withUnsafeBytes { Data($0) }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: keyId.uuidString,
            kSecValueData as String: keyData,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]

        SecItemDelete(query as CFDictionary)
        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw RelayError.serializationFailed("Keychain save failed with OSStatus \(status)")
        }
    }
}
