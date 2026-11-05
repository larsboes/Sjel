# Sjel macOS Companion & CloudKit Relay (`apps/mac`)

Native macOS menu bar companion and end-to-end encrypted CloudKit relay for Sjel, built in Swift.

The menu bar companion remains a small OS integration beside the GPUI app. The CloudKit relay is
implemented and tested but dormant: no current app or capability syncs through it.

## Architecture

Per Sjel / Sjel doctrine and **Product Rule 4**:
- **C2 Data Boundary**: Raw personally identifiable data (C2) never leaves the host without end-to-end encryption or pseudonymization.
- **CloudKit Storage Invariant**: Records written to Apple's private CloudKit database use an opaque generic record type (`SjelEncryptedRecord`) and opaque hashed record identifiers.
- **Zero Plaintext Leakage**: Stored CloudKit record dictionaries contain **zero** readable C2 field names and **zero** readable C2 values. The payload is sealed with authenticated AES-256-GCM (`CryptoKit.AES.GCM`) with keys stored strictly on user devices (Keychain / Secure Enclave).
- **Mechanical Sympathy**: Compact, native menu bar companion (`SjelMacApp`) displaying local daemon connectivity (`NodeBridge`), health latency, and CloudKit E2EE synchronization state. Notch extension or desktop surfaces remain optional add-ons rather than competing with full-featured desktop notch utilities (such as `vorssaint-utils`).

## Targets

- `SjelRelay`: Swift library managing `C2Record` modeling, `EncryptionEnvelope` AEAD packaging, `RelayKeyManager` Keychain storage, and `CloudKitRelay` translation to/from `CKRecord`.
- `SjelMacApp`: Native SwiftUI `MenuBarExtra` companion app: the shell's health, **Open
  App** (the GPUI desktop app, `dev.sjel.desktop`, installed by
  `apps/gpui/install`), and **Open at login**. When the installed desktop app is behind the
  checkout, a **Sjel Desktop update** row offers **Update**. It compares the `SjelBuildCommit`
  that `apps/gpui/install` writes into the bundle's Info.plist with `~/Developer/Sjel`.
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

## The inbound token

The menu bar reads the deployment's inbound token from the login Keychain (item
`sjel-inbound-token`, written by `tools/setup-inbound-auth.sh`) to call the shell for health and
agent approvals (`InboundToken`, `CachedToken`). The token never leaves this app.

## Agent writes that ask

In ask mode, and for an action that leaves Sjel or cannot be undone, an agent's write waits for
the owner. The app reads the waiting writes from the shell every five seconds. It lists
them in its menu with **Allow** and **Deny**, and posts a notification with the same two
buttons. The first notification asks macOS for permission to notify.

## Testing

```sh
cd apps/mac
swift test
```
