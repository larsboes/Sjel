import Foundation
import CloudKit
import CryptoKit

/// CloudKit relay responsible for end-to-end encrypted synchronization of Sjel records.
/// In strict compliance with Product Rule 4:
/// - CloudKit records stored in Apple's private database carry only opaque ciphertext envelopes.
/// - Zero C2 field names or values are readable in the stored CKRecord.
/// - Cryptographic keys never leave the owner's devices.
public final class CloudKitRelay: @unchecked Sendable {
    public static let recordType = "SjelEncryptedRecord"
    public static let fieldPayload = "payload"
    public static let fieldSchemaVersion = "schemaVersion"
    public static let fieldKeyEpoch = "keyEpoch"
    public static let fieldUpdatedAt = "updatedAt"

    private let keyManager: RelayKeyManager
    private let jsonEncoder: JSONEncoder
    private let jsonDecoder: JSONDecoder

    public init(keyManager: RelayKeyManager = .shared) {
        self.keyManager = keyManager
        self.jsonEncoder = JSONEncoder()
        self.jsonEncoder.outputFormatting = [.sortedKeys]
        self.jsonEncoder.dateEncodingStrategy = .iso8601

        self.jsonDecoder = JSONDecoder()
        self.jsonDecoder.dateDecodingStrategy = .iso8601
    }

    /// Computes an opaque, one-way record name for CloudKit to prevent leaking entity IDs or types.
    public static func opaqueRecordName(for entityType: String, id: String) -> String {
        let rawIdentifier = "\(entityType):\(id)"
        let digest = SHA256.hash(data: Data(rawIdentifier.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    /// Encrypts a C2 record into a CKRecord suitable for CloudKit storage.
    /// The resulting CKRecord contains ZERO readable C2 field names or values.
    public func encryptToCKRecord(
        c2Record: C2Record,
        key: SymmetricKey? = nil,
        keyId: UUID? = nil,
        zoneID: CKRecordZone.ID? = nil
    ) throws -> CKRecord {
        let (activeKeyId, activeKey): (UUID, SymmetricKey)
        if let key = key, let keyId = keyId {
            activeKeyId = keyId
            activeKey = key
        } else {
            (activeKeyId, activeKey) = try keyManager.getPrimaryKey()
        }

        // 1. Serialize the full C2 record to JSON
        let plaintextData = try jsonEncoder.encode(c2Record)

        // 2. Seal the data using authenticated AES-256-GCM
        let envelopeData = try EncryptionEnvelope.seal(
            plaintext: plaintextData,
            key: activeKey,
            keyId: activeKeyId
        )

        // 3. Construct the opaque CKRecord
        let recordName = Self.opaqueRecordName(for: c2Record.entityType, id: c2Record.id)
        let recordID: CKRecord.ID
        if let zone = zoneID {
            recordID = CKRecord.ID(recordName: recordName, zoneID: zone)
        } else {
            recordID = CKRecord.ID(recordName: recordName)
        }

        let ckRecord = CKRecord(recordType: Self.recordType, recordID: recordID)
        ckRecord[Self.fieldPayload] = envelopeData as CKRecordValue
        ckRecord[Self.fieldSchemaVersion] = 1 as CKRecordValue
        ckRecord[Self.fieldKeyEpoch] = 1 as CKRecordValue
        ckRecord[Self.fieldUpdatedAt] = c2Record.updatedAt as CKRecordValue

        return ckRecord
    }

    /// Decrypts a stored CKRecord back into its full C2Record representation.
    public func decryptFromCKRecord(
        record: CKRecord,
        key: SymmetricKey? = nil
    ) throws -> C2Record {
        guard record.recordType == Self.recordType else {
            throw RelayError.invalidEnvelopeHeader
        }

        guard let payloadData = record[Self.fieldPayload] as? Data else {
            throw RelayError.missingField(Self.fieldPayload)
        }

        let resolvedKey: SymmetricKey
        if let key = key {
            resolvedKey = key
        } else {
            let (_, fallbackKey) = try keyManager.getPrimaryKey()
            resolvedKey = fallbackKey
        }

        let (plaintextData, _) = try EncryptionEnvelope.open(
            envelopeData: payloadData,
            key: resolvedKey
        )

        let record = try jsonDecoder.decode(C2Record.self, from: plaintextData)
        return record
    }

    /// Pushes an encrypted C2Record to a CloudKit private database.
    public func upload(
        c2Record: C2Record,
        to database: CKDatabase,
        key: SymmetricKey? = nil,
        keyId: UUID? = nil
    ) async throws -> CKRecord {
        let record = try encryptToCKRecord(c2Record: c2Record, key: key, keyId: keyId)
        return try await database.save(record)
    }

    /// Fetches an encrypted record from CloudKit and restores the C2Record.
    public func fetch(
        entityType: String,
        id: String,
        from database: CKDatabase,
        key: SymmetricKey? = nil
    ) async throws -> C2Record {
        let recordName = Self.opaqueRecordName(for: entityType, id: id)
        let recordID = CKRecord.ID(recordName: recordName)
        let ckRecord = try await database.record(for: recordID)
        return try decryptFromCKRecord(record: ckRecord, key: key)
    }
}
