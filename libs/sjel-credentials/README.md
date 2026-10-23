# sjel-credentials

Provider-neutral credential references and secret operations for Sjel's operator tools. A
catalog names each credential and the store that holds it; the secret itself never enters the
catalog.

## What is here

- **`CredentialCatalog`** — the non-secret list: an ID, a label, and a provider reference per
  entry. It is TOML on disk and has no field that can hold a value.
- **`SecretValue`** — the bytes, in a buffer that is zeroed on drop. It does not implement
  `Debug`, `Display` or `Serialize`, so a value cannot end up in JSON or a log line by accident.
- **`CredentialManager`** — list, status, create, update, delete, and `materialize`. The last
  one writes a value to a mode-0600 runtime file through a temporary file and an atomic rename,
  for a launchd service that cannot unlock a Keychain (`src/manager.rs`).
- **`KeychainProvider`** — the macOS login Keychain, through the `security` command
  (`src/keychain.rs`).
- **`google`** — the one Google OAuth sign-in (loopback, PKCE, `prompt=consent`) and the one
  token refresh that comms, calendar and entities call (`src/google.rs`). A grant is one keychain
  item holding JSON; `access_token` reads the keychain first and the capability's old overlay
  env file second.
- **`declaration`** — `[[credentials]]` in a capability's `service.toml`: what it asks for and
  the steps to obtain it, never a value (`src/declaration.rs`, schemas/service.toml.example).
  sjel-status lists them at `GET /api/sjel-status/credentials`; the desktop app's
  Settings › Accounts page connects a grant in-process (`apps/gpui/core/src/accounts.rs`).

## Not built yet

`ProviderKind::Bitwarden` exists in the types, but no adapter implements it. Callers must not
return a `SecretValue` to a webview or put one in a diagnostic.
