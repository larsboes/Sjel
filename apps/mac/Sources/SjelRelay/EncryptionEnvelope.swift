import Foundation
import CryptoKit

public enum RelayError: Error, LocalizedError, Equatable {
    case invalidEnvelopeHeader
    case authenticationFailed
    case decryptionFailed(String)
    case serializationFailed(String)
    case missingField(String)
    case keyMismatch

    public var errorDescription: String? {
        switch self {
        case .invalidEnvelopeHeader:
            return "The encrypted envelope header is invalid or unrecognized."
        case .authenticationFailed:
            return "Cryptographic authentication failed. Data may have been tampered with or key is incorrect."
        case .decryptionFailed(let msg):
            return "Decryption failed: \(msg)"
        case .serializationFailed(let msg):
            return "Serialization failed: \(msg)"
        case .missingField(let name):
            return "Required field '\(name)' missing from stored record."
        case .keyMismatch:
            return "Encryption key does not match the key ID in the envelope header."
        }
    }
}

/// Binary envelope providing AEAD encryption (AES-256-GCM) with key identifier framing.
public struct EncryptionEnvelope: Sendable {
    public static let magic = Data([0x53, 0x4A, 0x45, 0x4C]) // "SJEL"
    public static let currentVersion: UInt8 = 1

    /// Seals plaintext data with AES-256-GCM and prepends envelope header.
    /// Format:
    /// [4 bytes: "SJEL"] [1 byte: version] [16 bytes: keyId UUID] [combined AES.GCM: 12-byte nonce + ciphertext + 16-byte tag]
    public static func seal(
        plaintext: Data,
        key: SymmetricKey,
        keyId: UUID
    ) throws -> Data {
        let sealedBox = try AES.GCM.seal(plaintext, using: key)
        guard let combined = sealedBox.combined else {
            throw RelayError.serializationFailed("Failed to generate combined AES.GCM sealed box")
        }

        var envelope = Data()
        envelope.reserveCapacity(4 + 1 + 16 + combined.count)
        envelope.append(magic)
        envelope.append(currentVersion)

        var uuidBytes = keyId.uuid
        withUnsafeBytes(of: &uuidBytes) { rawBytes in
            envelope.append(contentsOf: rawBytes)
        }

        envelope.append(combined)
        return envelope
    }

    /// Opens an encrypted envelope data blob, verifying the AEAD tag and extracting plaintext.
    public static func open(
        envelopeData: Data,
        key: SymmetricKey,
        expectedKeyId: UUID? = nil
    ) throws -> (plaintext: Data, keyId: UUID) {
        let minLength = 4 + 1 + 16 + 12 + 16 // magic + version + keyId + nonce + tag
        guard envelopeData.count >= minLength else {
            throw RelayError.invalidEnvelopeHeader
        }

        guard envelopeData.prefix(4) == magic else {
            throw RelayError.invalidEnvelopeHeader
        }

        let version = envelopeData[4]
        guard version == currentVersion else {
            throw RelayError.invalidEnvelopeHeader
        }

        let uuidData = envelopeData.subdata(in: 5..<21)
        let keyId = uuidData.withUnsafeBytes { raw in
            raw.load(as: UUID.self)
        }

        if let expected = expectedKeyId, expected != keyId {
            throw RelayError.keyMismatch
        }

        let combinedCiphertext = envelopeData.subdata(in: 21..<envelopeData.count)

        do {
            let sealedBox = try AES.GCM.SealedBox(combined: combinedCiphertext)
            let plaintext = try AES.GCM.open(sealedBox, using: key)
            return (plaintext, keyId)
        } catch {
            throw RelayError.authenticationFailed
        }
    }
}
