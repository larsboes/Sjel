import CryptoKit
import Foundation
import Security

// The device key for `axon-device-auth/v1`. A port of plugins/device-identity (the Tauri app's
// identity) under its own Keychain service: a Curve25519 signing key, created once, kept
// `ThisDeviceOnly`. Rust gets the public key and signatures over C; the private key never
// leaves this file.

private let keychainService = "dev.sjel.ios.device-identity"
private let keychainAccount = "axon-ed25519-signing-key"  // gitleaks:allow — a Keychain item label, not key material

private func loadOrCreateKey() -> Curve25519.Signing.PrivateKey? {
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: keychainService,
        kSecAttrAccount as String: keychainAccount,
        kSecReturnData as String: true,
        kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    var item: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &item)
    if status == errSecSuccess {
        guard let data = item as? Data else { return nil }
        return try? Curve25519.Signing.PrivateKey(rawRepresentation: data)
    }
    guard status == errSecItemNotFound else { return nil }

    let key = Curve25519.Signing.PrivateKey()
    let attributes: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: keychainService,
        kSecAttrAccount as String: keychainAccount,
        kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        kSecValueData as String: key.rawRepresentation,
    ]
    let written = SecItemAdd(attributes as CFDictionary, nil)
    if written == errSecDuplicateItem { return loadOrCreateKey() }
    return written == errSecSuccess ? key : nil
}

/// Writes the 32-byte public key to `out`. Returns 0, or -1 when Keychain refused.
@_cdecl("sjel_device_public_key")
public func sjelDevicePublicKey(_ out: UnsafeMutablePointer<UInt8>) -> Int32 {
    guard let key = loadOrCreateKey() else { return -1 }
    let bytes = key.publicKey.rawRepresentation
    guard bytes.count == 32 else { return -1 }
    bytes.copyBytes(to: out, count: 32)
    return 0
}

/// Signs `length` bytes at `message` and writes the 64-byte signature to `out`. Returns 0, or -1.
@_cdecl("sjel_device_sign")
public func sjelDeviceSign(
    _ message: UnsafePointer<UInt8>, _ length: Int, _ out: UnsafeMutablePointer<UInt8>
) -> Int32 {
    guard let key = loadOrCreateKey(),
          let signature = try? key.signature(for: Data(bytes: message, count: length)),
          signature.count == 64
    else { return -1 }
    signature.copyBytes(to: out, count: 64)
    return 0
}
