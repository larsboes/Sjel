import Testing
import Foundation
import CloudKit
import CryptoKit
@testable import SjelRelay

@Suite("CloudKit Relay & Product Rule 4 Verification")
struct CloudKitRelayTests {

    /// ISC-19 Falsifier Test:
    /// "a test reads a record back as Apple stores it and finds ciphertext (product rule 4).
    /// Falsifier: a field name or value of a C2 record readable in the stored record."
    @Test("C2 record in CloudKit storage is strictly ciphertext with zero readable C2 fields or values")
    func c2RecordInCloudKitStorageHasZeroReadableFieldsOrValues() throws {
        let keyManager = RelayKeyManager()
        let (keyId, key) = try keyManager.generateAndStoreKey()
        let relay = CloudKitRelay(keyManager: keyManager)

        // 1. Create a C2 record containing sensitive personal information
        let sensitiveFields: [String: C2Value] = [
            "fullName": .string("Alice Wonderly"),
            "emailAddress": .string("alice.wonderly@confidential-corp.internal"),
            "phoneNumber": .string("+49-170-98765432"),
            "medicalNotes": .string("Diagnosed with acute migraine; prescribed sumatriptan 50mg"),
            "bankIban": .string("DE44500105175407324931"),
            "netWorthAmount": .number(842500.75),
            "ssn": .string("987-65-4321")
        ]

        let c2Record = C2Record(
            id: "person-alice-3914",
            entityType: "person",
            createdAt: Date(timeIntervalSince1970: 1700000000),
            updatedAt: Date(timeIntervalSince1970: 1700000100),
            attributes: sensitiveFields
        )

        // 2. Encrypt into CKRecord as it would be delivered to Apple's CloudKit servers
        let ckRecord = try relay.encryptToCKRecord(c2Record: c2Record, key: key, keyId: keyId)

        // --- ASSERTION 1: The recordType must NOT be the entity type ("person") ---
        #expect(ckRecord.recordType == "SjelEncryptedRecord")
        #expect(ckRecord.recordType != "person")

        // --- ASSERTION 2: The recordID must NOT leak the entity ID or name ---
        let recordName = ckRecord.recordID.recordName
        #expect(!recordName.contains("alice"))
        #expect(!recordName.contains("person-alice-3914"))
        #expect(!recordName.contains("person"))

        // --- ASSERTION 3: Stored record keys must contain ZERO C2 field names ---
        let storedKeys = Set(ckRecord.allKeys())
        let c2FieldNames = [
            "fullName",
            "emailAddress",
            "phoneNumber",
            "medicalNotes",
            "bankIban",
            "netWorthAmount",
            "ssn"
        ]

        for field in c2FieldNames {
            #expect(!storedKeys.contains(field), "FALSIFIER VIOLATION: C2 field name '\(field)' is readable in stored CKRecord keys!")
        }

        // Only permitted envelope metadata fields may exist
        let permittedEnvelopeKeys: Set<String> = [
            CloudKitRelay.fieldPayload,
            CloudKitRelay.fieldSchemaVersion,
            CloudKitRelay.fieldKeyEpoch,
            CloudKitRelay.fieldUpdatedAt
        ]
        #expect(storedKeys.isSubset(of: permittedEnvelopeKeys))

        // --- ASSERTION 4: Stored values must contain ZERO C2 plaintext values ---
        let forbiddenPlaintextSubstrings = [
            "Alice",
            "Wonderly",
            "confidential-corp",
            "98765432",
            "migraine",
            "sumatriptan",
            "DE44500105175407324931",
            "842500",
            "987-65-4321"
        ]

        // Inspect every value stored on the CKRecord
        for key in storedKeys {
            if let strVal = ckRecord[key] as? String {
                for forbidden in forbiddenPlaintextSubstrings {
                    #expect(!strVal.localizedCaseInsensitiveContains(forbidden),
                            "FALSIFIER VIOLATION: Plaintext value '\(forbidden)' leaked in CKRecord key '\(key)'!")
                }
            }
        }

        // Inspect raw record description / serialization
        let recordDescription = ckRecord.description
        let recordDebugDescription = ckRecord.debugDescription
        for forbidden in forbiddenPlaintextSubstrings {
            #expect(!recordDescription.localizedCaseInsensitiveContains(forbidden),
                    "FALSIFIER VIOLATION: Plaintext value '\(forbidden)' found in CKRecord description!")
            #expect(!recordDebugDescription.localizedCaseInsensitiveContains(forbidden),
                    "FALSIFIER VIOLATION: Plaintext value '\(forbidden)' found in CKRecord debugDescription!")
        }

        // Inspect the payload blob itself
        guard let payloadData = ckRecord[CloudKitRelay.fieldPayload] as? Data else {
            Issue.record("Missing payload data in CKRecord")
            return
        }

        #expect(payloadData.prefix(4) == EncryptionEnvelope.magic)

        // Ensure the payload data does NOT contain plaintext ASCII/UTF-8 strings
        let payloadString = String(decoding: payloadData, as: UTF8.self)
        for forbidden in forbiddenPlaintextSubstrings {
            #expect(!payloadString.localizedCaseInsensitiveContains(forbidden),
                    "FALSIFIER VIOLATION: Raw payload data contains plaintext string '\(forbidden)'!")
        }

        // --- ASSERTION 5: Full roundtrip decryption restores original C2 record ---
        let decrypted = try relay.decryptFromCKRecord(record: ckRecord, key: key)
        #expect(decrypted.id == c2Record.id)
        #expect(decrypted.entityType == c2Record.entityType)
        #expect(decrypted.attributes == c2Record.attributes)
        #expect(decrypted.attributes["fullName"] == .string("Alice Wonderly"))
        #expect(decrypted.attributes["bankIban"] == .string("DE44500105175407324931"))
    }

    @Test("Tampered ciphertext refuses decryption and raises authentication failure")
    func tamperedCiphertextRefusesDecryption() throws {
        let keyManager = RelayKeyManager()
        let (keyId, key) = try keyManager.generateAndStoreKey()
        let relay = CloudKitRelay(keyManager: keyManager)

        let record = C2Record(
            id: "tx-123",
            entityType: "transaction",
            attributes: ["amount": .number(99.99), "vendor": .string("Hardware Store")]
        )

        let ckRecord = try relay.encryptToCKRecord(c2Record: record, key: key, keyId: keyId)
        guard var payloadData = ckRecord[CloudKitRelay.fieldPayload] as? Data else {
            Issue.record("Missing payload data")
            return
        }

        // Flip a byte in the ciphertext body
        let tamperIndex = payloadData.count - 5
        payloadData[tamperIndex] ^= 0xFF
        ckRecord[CloudKitRelay.fieldPayload] = payloadData as CKRecordValue

        #expect(throws: RelayError.authenticationFailed) {
            try relay.decryptFromCKRecord(record: ckRecord, key: key)
        }
    }

    @Test("Decryption with incorrect symmetric key raises authentication failure")
    func wrongKeyRefusesDecryption() throws {
        let keyManager = RelayKeyManager()
        let (keyId, key1) = try keyManager.generateAndStoreKey()
        let (_, key2) = try keyManager.generateAndStoreKey()
        let relay = CloudKitRelay(keyManager: keyManager)

        let record = C2Record(
            id: "msg-456",
            entityType: "message",
            attributes: ["text": .string("Private message")]
        )

        let ckRecord = try relay.encryptToCKRecord(c2Record: record, key: key1, keyId: keyId)

        #expect(throws: RelayError.authenticationFailed) {
            try relay.decryptFromCKRecord(record: ckRecord, key: key2)
        }
    }
}
