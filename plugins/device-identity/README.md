# Tauri device identity plugin

This plugin is the iOS half of Sjel's `axon-pairing/v1` enrollment flow.

- The first invocation creates a Curve25519 signing key in iOS Keychain.
- Later invocations load the same key.
- The Keychain item is `ThisDeviceOnly` and available after first unlock.
- The plugin returns a stable local ID, `ed25519`, `ios`, and the public key in hexadecimal.
- Request bytes may be sent to the native plugin and a hexadecimal signature returned; the private key never crosses Swift, Rust, or the WebView.
- An explicit re-pairing action can replace the Keychain identity; the old public key remains revoked at the node.

Desktop and Android builds intentionally do not provide a software-key fallback. The Rust bridge
is mobile-gated and the dashboard claim form only appears in an iOS Tauri WebView.
