import CryptoKit
import Foundation
import Security
import Tauri

private let keychainService = "com.larsboes.sjel.device-identity"
private let keychainAccount = "axon-ed25519-signing-key"  // gitleaks:allow — a Keychain item label, not key material

private struct IdentityPayload: Encodable {
  let id: String
  let platform = "ios"
  let algorithm = "ed25519"
  let publicKey: String

  enum CodingKeys: String, CodingKey {
    case id
    case platform
    case algorithm
    case publicKey = "public_key"
  }
}

private struct SignArguments: Decodable {
  let message: String
}

private struct SignaturePayload: Encodable {
  let signature: String
}

private enum DeviceIdentityError: LocalizedError {
  case keychainRead(OSStatus)
  case keychainWrite(OSStatus)
  case keychainDelete(OSStatus)
  case invalidStoredKey

  var errorDescription: String? {
    switch self {
    case .keychainRead(let status):
      return "Could not read the Sjel signing key from Keychain (status \(status))."
    case .keychainWrite(let status):
      return "Could not store the Sjel signing key in Keychain (status \(status))."
    case .keychainDelete(let status):
      return "Could not remove the Sjel signing key from Keychain (status \(status))."
    case .invalidStoredKey:
      return "The Sjel signing key in Keychain is invalid."
    }
  }
}

final class DeviceIdentityPlugin: Plugin {
  @objc public func getIdentity(_ invoke: Invoke) {
    do {
      let key = try loadOrCreateKey()
      let publicKey = key.publicKey.rawRepresentation
      let digest = SHA256.hash(data: publicKey)
      let identity = IdentityPayload(
        id: "dev_\(digest.hexString.prefix(32))",
        publicKey: publicKey.hexString
      )
      invoke.resolve(identity)
    } catch {
      invoke.reject(error.localizedDescription)
    }
  }

  @objc public func sign(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(SignArguments.self)
      guard let message = Data(hex: args.message) else {
        invoke.reject("The signing message must be hexadecimal.")
        return
      }
      let signature = try loadOrCreateKey().signature(for: message)
      invoke.resolve(SignaturePayload(signature: signature.hexString))
    } catch {
      invoke.reject(error.localizedDescription)
    }
  }

  @objc public func resetIdentity(_ invoke: Invoke) {
    do {
      try deleteStoredKey()
      try getIdentity(invoke)
    } catch {
      invoke.reject(error.localizedDescription)
    }
  }

  private func deleteStoredKey() throws {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: keychainService,
      kSecAttrAccount as String: keychainAccount
    ]
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw DeviceIdentityError.keychainDelete(status)
    }
  }

  private func loadOrCreateKey() throws -> Curve25519.Signing.PrivateKey {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: keychainService,
      kSecAttrAccount as String: keychainAccount,
      kSecReturnData as String: true,
      kSecMatchLimit as String: kSecMatchLimitOne
    ]

    var item: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &item)
    if status == errSecSuccess {
      guard let data = item as? Data else {
        throw DeviceIdentityError.invalidStoredKey
      }
      do {
        return try Curve25519.Signing.PrivateKey(rawRepresentation: data)
      } catch {
        throw DeviceIdentityError.invalidStoredKey
      }
    }
    if status != errSecItemNotFound {
      throw DeviceIdentityError.keychainRead(status)
    }

    let key = Curve25519.Signing.PrivateKey()
    let attributes: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: keychainService,
      kSecAttrAccount as String: keychainAccount,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
      kSecValueData as String: key.rawRepresentation
    ]
    let writeStatus = SecItemAdd(attributes as CFDictionary, nil)
    if writeStatus == errSecDuplicateItem {
      // Another app invocation won the create race. Read its key so the identity stays stable.
      return try loadOrCreateKey()
    }
    guard writeStatus == errSecSuccess else {
      throw DeviceIdentityError.keychainWrite(writeStatus)
    }
    return key
  }
}

private extension Data {
  init?(hex: String) {
    guard hex.count.isMultiple(of: 2) else { return nil }
    var bytes = [UInt8]()
    bytes.reserveCapacity(hex.count / 2)
    var index = hex.startIndex
    while index < hex.endIndex {
      let next = hex.index(index, offsetBy: 2)
      guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
      bytes.append(byte)
      index = next
    }
    self.init(bytes)
  }
}

private extension Sequence where Element == UInt8 {
  var hexString: String {
    map { String(format: "%02x", $0) }.joined()
  }
}

@_cdecl("init_plugin_device_identity")
func initPlugin() -> Plugin {
  DeviceIdentityPlugin()
}
