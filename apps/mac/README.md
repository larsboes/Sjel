# Sjel macOS Companion & CloudKit Relay (`apps/mac`)

Native macOS menu bar companion and end-to-end encrypted CloudKit relay for Sjel, built in Swift.

## Architecture

Per Sjel / Sjel doctrine and **Product Rule 4**:
- **C2 Data Boundary**: Raw personally identifiable data (C2) never leaves the host without end-to-end encryption or pseudonymization.
- **CloudKit Storage Invariant**: Records written to Apple's private CloudKit database use an opaque generic record type (`SjelEncryptedRecord`) and opaque hashed record identifiers.
- **Zero Plaintext Leakage**: Stored CloudKit record dictionaries contain **zero** readable C2 field names and **zero** readable C2 values. The payload is sealed with authenticated AES-256-GCM (`CryptoKit.AES.GCM`) with keys stored strictly on user devices (Keychain / Secure Enclave).
- **Mechanical Sympathy**: Compact, native menu bar companion (`SjelMacApp`) displaying local daemon connectivity (`NodeBridge`), health latency, and CloudKit E2EE synchronization state. Notch extension or desktop surfaces remain optional add-ons rather than competing with full-featured desktop notch utilities (such as `vorssaint-utils`).

## Targets

- `SjelRelay`: Swift library managing `C2Record` modeling, `EncryptionEnvelope` AEAD packaging, `RelayKeyManager` Keychain storage, and `CloudKitRelay` translation to/from `CKRecord`.
- `SjelMacApp`: Native SwiftUI `MenuBarExtra` companion app: the shell's health, **Open
  Dashboard**, and **Open at login**.
- `SjelRelayTests`: Test suite verifying the falsifier:
  - `c2RecordInCloudKitStorageHasZeroReadableFieldsOrValues`: Asserts that stored `CKRecord.allKeys()` and all record values contain no C2 field names or values, and verifies full roundtrip decryption.
  - `tamperedCiphertextRefusesDecryption`: Asserts AEAD authentication rejects modified payloads.
  - `wrongKeyRefusesDecryption`: Asserts foreign keys fail authentication.

## Install

```sh
apps/mac/install
```

Builds a signed `~/Applications/Sjel.app` and starts it. It lives in the menu bar only. Its menu
has **Open at login**, so the app is there after a restart.

## Opening the dashboard

The shell at `127.0.0.1:8082` refuses a browser that carries no credential (ISA ISC-45).
**Open Dashboard** reads the inbound token from the login Keychain (item `sjel-inbound-token`,
written by `tools/setup-inbound-auth.sh`). It trades the token for a single-use ticket and opens
the browser with it. The shell turns the ticket into a 30-day session cookie
(`capabilities/sjel-status/src/session.rs`). The token never reaches the browser.

## Testing

```sh
cd apps/mac
swift test
```
