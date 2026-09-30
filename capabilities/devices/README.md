# devices

The deployment's device registry. It owns one-time pairing challenges, registered device public
keys, signed-request replay state and revocation state. It does not own capability data or relay
transport.

## Contract

- `POST /api/pairing/challenges` creates a ten-minute challenge. The plaintext code is returned
  once; only its SHA-256 digest is stored.
- `POST /api/pairing/claims` consumes a challenge and registers one Ed25519 public key. The private
  key is never sent to Sjel.
- `GET /api/devices` lists active and revoked devices.
- `POST /api/devices/:id/revoke` revokes one active device.
- `POST /api/pairing/requests` is a device asking to join with its label and public key. It waits,
  `pending`, for ten minutes. At most five wait at once.
- `GET /api/pairing/requests` lists the waiting requests, each with `key_code`: six digits from
  the requesting key. `GET /api/pairing/requests/:id` is one request's state, which the device polls.
- `POST /api/pairing/requests/:id/approve` registers the key the device asked with.
  `POST /api/pairing/requests/:id/deny` refuses it.
- `GET /api/devices/me` requires `axon-device-auth/v1` headers and returns the authenticated device.

Signed requests use `X-Sjel-Device-Id`, `X-Sjel-Timestamp`, `X-Sjel-Nonce` and
`X-Sjel-Signature`. The Ed25519 signature covers the protocol version, device id, timestamp,
nonce, uppercase method, capability path and SHA-256 body digest. Timestamps must be within five
minutes of the node clock. Accepted nonces are retained for ten minutes and are unique per device;
revocation is checked again at the nonce commit.

The Sjel status shell also admits a request on a valid device signature (PRD Q119: the device
key, not the network, is the trust root). Its gate (`libs/sjel-server/src/auth.rs`,
`with_device_verifier`) checks every request that carries `X-Sjel-Signature` against this registry
through `DevicesStore::authenticate_scoped`, before it reaches any capability. A valid signature
admits the request and the shell sends the deployment token upstream; an invalid one is refused
with `401` and never falls through to the tailnet or token rules. The shell consumes the nonce in
its own `shell` scope, so the same request is not a replay when it reaches `/api/devices/me`.

## Guided pairing

The iPhone app opens "Set up Sjel" until it is registered (`dashboard/src/lib/setup`). The Mac's
Devices panel shows a QR code with the node's Same Wi-Fi address, its certificate fingerprint and
a one-time code; one scan pins the Mac and claims the code. Without a code, the phone finds the
Mac over Bonjour and asks to join, and the Mac's dashboard asks the owner to allow it. Both screens
show the same code: the Mac's certificate half, which a relay cannot forge, and the key half,
which tells this phone's request from any other.

On the LAN listener an unregistered device may send exactly three unsigned requests: a claim, a
join request, and one join request's state (`libs/sjel-server/src/auth.rs`,
`unsigned_pairing_route`). The phone leaves those three unsigned (`dashboard/src-tauri/src/mac_bridge.rs`,
`unsigned_pairing_request`); every other request is signed.

The service is reached through the Sjel status shell at `/devices/api/...`. It remains behind the
existing deployment inbound gate and origin guard. Pairing and operator registry routes remain
protected by that deployment gate; device data routes use the signed device identity as well.

The store uses the shared Sjel SQLite file under the `devices_` prefix and is covered by the service
backup contract. Challenge rows are deliberately retained after use so replay attempts can be
reported as "already used" rather than looking like missing state.
